use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

use crate::shell;

mod utf8;
use utf8::Utf8Chunker;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Événements envoyés au frontend pour une session.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PtyEvent {
    Data { chunk: String },
    Exit { code: Option<u32> },
    Error { message: String },
}

pub struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Session {
    pub fn write(&self, data: &[u8]) -> std::io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("writer verrouillé"))?;
        writer.write_all(data)?;
        writer.flush()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("redimensionnement du pty")
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }
}

pub fn next_session_id() -> String {
    format!("pty-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// Lance `shell` dans un pseudo-terminal. `tap` voit la sortie brute et peut
/// la transformer (capture des commandes) avant l'envoi à `emit`.
pub fn spawn<F, T>(
    shell: &str,
    cwd: Option<&str>,
    cols: u16,
    rows: u16,
    state_dir: &Path,
    mut tap: T,
    emit: F,
) -> Result<Session>
where
    F: Fn(PtyEvent) -> bool + Clone + Send + 'static,
    T: FnMut(&str) -> String + Send + 'static,
{
    let pair = native_pty_system().openpty(PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new(shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if let Some(cwd) = cwd.filter(|c| !c.is_empty()) {
        cmd.cwd(cwd);
    }

    match shell::detect(shell) {
        Some(integration) => {
            let dir = state_dir.join("shell").join(integration.id());
            integration
                .install(&dir)
                .context("installation de l'intégration shell")?;
            for (key, value) in integration.env(&dir) {
                cmd.env(key, value);
            }
            for arg in integration.args() {
                cmd.arg(arg);
            }
        }
        None => log::warn!("aucune intégration shell pour {shell} : capture désactivée"),
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("spawn de {shell}"))?;
    // Le slave doit être fermé côté parent, sinon le reader ne voit jamais EOF.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let killer = child.clone_killer();

    let data_emit = emit.clone();
    thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut chunker = Utf8Chunker::default();
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = chunker.push(&buf[..n]);
                        let visible = tap(&text);
                        if !visible.is_empty()
                            && !data_emit(PtyEvent::Data { chunk: visible })
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = data_emit(PtyEvent::Error { message: e.to_string() });
                        break;
                    }
                }
            }
            let tail = chunker.flush();
            if !tail.is_empty() {
                let visible = tap(&tail);
                if !visible.is_empty() {
                    let _ = data_emit(PtyEvent::Data { chunk: visible });
                }
            }
        })?;

    thread::Builder::new()
        .name("pty-waiter".into())
        .spawn(move || {
            let code = child.wait().ok().map(|status| status.exit_code());
            let _ = emit(PtyEvent::Exit { code });
        })?;

    Ok(Session {
        master: pair.master,
        writer: Mutex::new(writer),
        killer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{Capture, CommandRecord};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};

    #[test]
    fn lance_un_shell_et_recupere_la_sortie() {
        let output = Arc::new(StdMutex::new(String::new()));
        let sink = output.clone();
        let emit = move |event: PtyEvent| {
            if let PtyEvent::Data { chunk } = event {
                sink.lock().unwrap().push_str(&chunk);
            }
            true
        };

        let state_dir = std::env::temp_dir();
        let mut session = spawn(
            "/bin/sh",
            None,
            80,
            24,
            &state_dir,
            |text: &str| text.to_string(),
            emit,
        )
        .expect("spawn du shell");
        session.write(b"echo hello-pty\n").expect("écriture");

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if output.lock().unwrap().contains("hello-pty") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        session.kill();
        let seen = output.lock().unwrap().clone();
        assert!(seen.contains("hello-pty"), "sortie reçue : {seen:?}");
    }

    #[test]
    fn zsh_emet_les_marqueurs_de_capture() {
        if !Path::new("/usr/bin/zsh").exists() {
            return;
        }

        let state_dir = std::env::temp_dir().join("mnemors-test-capture");
        std::fs::create_dir_all(&state_dir).unwrap();

        let records = Arc::new(StdMutex::new(Vec::<CommandRecord>::new()));
        let sink = records.clone();
        let mut capture = Capture::default();
        let tap = move |text: &str| {
            let (visible, captured) = capture.feed(text);
            sink.lock().unwrap().extend(captured);
            visible
        };

        let mut session = spawn(
            "/usr/bin/zsh",
            None,
            80,
            24,
            &state_dir,
            tap,
            |_| true,
        )
        .expect("spawn de zsh");

        session.write(b"echo hello-capture\n").expect("écriture");

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !records.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        session.kill();
        std::fs::remove_dir_all(&state_dir).ok();

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1, "enregistrements : {records:?}");
        assert_eq!(records[0].cmd, "echo hello-capture");
        assert_eq!(records[0].exit_code, 0);
        assert!(records[0].output.contains("hello-capture"));
    }

    #[test]
    fn commande_sans_sortie_n_enregistre_rien() {
        if !Path::new("/usr/bin/zsh").exists() {
            return;
        }

        let state_dir = std::env::temp_dir().join("mnemors-test-empty");
        std::fs::create_dir_all(&state_dir).unwrap();

        let records = Arc::new(StdMutex::new(Vec::<CommandRecord>::new()));
        let sink = records.clone();
        let mut capture = Capture::default();
        let tap = move |text: &str| {
            let (visible, captured) = capture.feed(text);
            sink.lock().unwrap().extend(captured);
            visible
        };

        let mut session = spawn(
            "/usr/bin/zsh",
            None,
            80,
            24,
            &state_dir,
            tap,
            |_| true,
        )
        .expect("spawn de zsh");

        session.write(b"false\n").expect("écriture");

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !records.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        session.kill();
        std::fs::remove_dir_all(&state_dir).ok();

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1, "enregistrements : {records:?}");
        assert_eq!(records[0].exit_code, 1);
        assert_eq!(records[0].output, "");
    }

    #[test]
    fn ctrl_c_produit_le_code_130() {
        if !Path::new("/usr/bin/zsh").exists() {
            return;
        }

        let state_dir = std::env::temp_dir().join("mnemors-test-ctrl-c");
        std::fs::create_dir_all(&state_dir).unwrap();

        let records = Arc::new(StdMutex::new(Vec::<CommandRecord>::new()));
        let sink = records.clone();
        let mut capture = Capture::default();
        let tap = move |text: &str| {
            let (visible, captured) = capture.feed(text);
            sink.lock().unwrap().extend(captured);
            visible
        };

        let mut session = spawn(
            "/usr/bin/zsh",
            None,
            80,
            24,
            &state_dir,
            tap,
            |_| true,
        )
        .expect("spawn de zsh");

        session.write(b"sleep 30\n").expect("écriture");
        std::thread::sleep(Duration::from_millis(300));
        session.write(b"\x03").expect("Ctrl-C");

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !records.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        session.kill();
        std::fs::remove_dir_all(&state_dir).ok();

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1, "enregistrements : {records:?}");
        assert_eq!(records[0].cmd, "sleep 30");
        assert_eq!(records[0].exit_code, 130);
    }
}
