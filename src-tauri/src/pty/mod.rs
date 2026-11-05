use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

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

/// Lance `shell` dans un pseudo-terminal et émet sa sortie via `emit`.
/// `emit` est découplé de Tauri pour permettre de tester le PTY seul.
pub fn spawn<F>(
    shell: &str,
    cwd: Option<&str>,
    cols: u16,
    rows: u16,
    emit: F,
) -> Result<(String, Session)>
where
    F: Fn(PtyEvent) -> bool + Clone + Send + 'static,
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
                        if !text.is_empty() && !data_emit(PtyEvent::Data { chunk: text }) {
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
                let _ = data_emit(PtyEvent::Data { chunk: tail });
            }
        })?;

    thread::Builder::new()
        .name("pty-waiter".into())
        .spawn(move || {
            let code = child.wait().ok().map(|status| status.exit_code());
            let _ = emit(PtyEvent::Exit { code });
        })?;

    let id = format!("pty-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
    Ok((
        id,
        Session {
            master: pair.master,
            writer: Mutex::new(writer),
            killer,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let (_, mut session) = spawn("/bin/sh", None, 80, 24, emit).expect("spawn du shell");
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
}
