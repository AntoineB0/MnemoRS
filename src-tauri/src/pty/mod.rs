use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::ipc::Channel;

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

/// Lance `shell` dans un pseudo-terminal et pousse sa sortie dans `channel`.
pub fn spawn(
    shell: &str,
    cwd: Option<&str>,
    cols: u16,
    rows: u16,
    channel: Channel<PtyEvent>,
) -> Result<(String, Session)> {
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

    let data_channel = channel.clone();
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
                        if !text.is_empty()
                            && data_channel.send(PtyEvent::Data { chunk: text }).is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = data_channel.send(PtyEvent::Error {
                            message: e.to_string(),
                        });
                        break;
                    }
                }
            }
            let tail = chunker.flush();
            if !tail.is_empty() {
                let _ = data_channel.send(PtyEvent::Data { chunk: tail });
            }
        })?;

    thread::Builder::new()
        .name("pty-waiter".into())
        .spawn(move || {
            let code = child.wait().ok().map(|status| status.exit_code());
            let _ = channel.send(PtyEvent::Exit { code });
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
