use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::clean_text;

const MAN_TIMEOUT: Duration = Duration::from_secs(5);
const HELP_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Outils dont l'aide est inutile, interactive ou dangereuse à lancer.
const SKIP_TOOLS: &[&str] = &[
    "sudo", "su", "doas", "passwd", "vim", "vi", "nano", "less", "more", "top", "htop", "watch",
    "man", "cd", "echo", "pwd", "exit", "clear", "history", "alias", "export", "source", "true",
    "false", "sleep", "which", "type", "rm", "mv", "cp", "mkdir", "touch", "kill", "pkill",
];

pub struct SourceDoc {
    pub source: String,
    pub tool: Option<String>,
    pub text: String,
}

/// Rassemble la documentation des outils de l'historique et les READMEs du
/// dossier. Chaque source est nettoyée et prête à être découpée.
pub fn collect(tools: &[String], dir: &Path) -> Vec<SourceDoc> {
    let mut docs = Vec::new();

    for tool in tools {
        if SKIP_TOOLS.contains(&tool.as_str()) {
            continue;
        }
        let text = man_page(tool)
            .or_else(|| help_text(tool))
            .map(|text| clean_text(&text));
        if let Some(text) = text.filter(|text| text.trim().len() > 200) {
            docs.push(SourceDoc {
                source: format!("doc:{tool}"),
                tool: Some(tool.clone()),
                text,
            });
        }
    }

    docs.extend(readmes(dir));
    docs
}

pub fn man_page(tool: &str) -> Option<String> {
    run_with_timeout("man", &["-P", "cat", "--", tool], MAN_TIMEOUT)
}

pub fn help_text(tool: &str) -> Option<String> {
    run_with_timeout(tool, &["--help"], HELP_TIMEOUT)
        .or_else(|| run_with_timeout(tool, &["-h"], HELP_TIMEOUT))
}

/// Fichiers Markdown du dossier (profondeur 2), hors dossiers de build.
pub fn readmes(dir: &Path) -> Vec<SourceDoc> {
    let mut files = Vec::new();
    walk(dir, 0, &mut files);
    files.sort();

    files
        .into_iter()
        .filter_map(|path| {
            let size = std::fs::metadata(&path).ok()?.len();
            if size == 0 || size > MAX_FILE_BYTES {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            Some(SourceDoc {
                source: path.display().to_string(),
                tool: None,
                text: clean_text(&text),
            })
        })
        .collect()
}

fn walk(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    if depth > 2 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if matches!(
                name.as_str(),
                "node_modules" | "target" | ".git" | "dist" | "build" | ".venv" | "venv" | ".cache"
            ) {
                continue;
            }
            walk(&path, depth + 1, files);
            continue;
        }

        let lower = name.to_lowercase();
        let is_readme = lower.starts_with("readme");
        let is_markdown = lower.ends_with(".md") || lower.ends_with(".markdown");
        if is_readme || is_markdown {
            files.push(path);
        }
    }
}

/// Lance une commande avec un délai maximal, sans bloquer sur les pipes :
/// stdout et stderr sont lus dans des threads dédiés.
fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let stdout = child.stdout.take()?;
    let stderr = child.stderr.take()?;
    let out_reader = thread::spawn(move || read_all(stdout));
    let err_reader = thread::spawn(move || read_all(stderr));

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }

    let stdout = out_reader.join().ok()?;
    let stderr = err_reader.join().ok()?;
    let combined = format!("{stdout}\n{stderr}").trim().to_string();
    (!combined.is_empty()).then_some(combined)
}

fn read_all(mut reader: impl Read) -> String {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_la_sortie() {
        let text = run_with_timeout("sh", &["-c", "echo bonjour"], Duration::from_secs(2)).unwrap();
        assert_eq!(text, "bonjour");
    }

    #[test]
    fn capture_stderr() {
        let text = run_with_timeout("sh", &["-c", "echo erreur >&2"], Duration::from_secs(2)).unwrap();
        assert_eq!(text, "erreur");
    }

    #[test]
    fn tue_les_commandes_trop_lentes() {
        let start = Instant::now();
        let text = run_with_timeout("sh", &["-c", "sleep 10"], Duration::from_millis(300));
        let elapsed = start.elapsed();
        assert!(text.is_none());
        assert!(elapsed < Duration::from_secs(3), "durée {elapsed:?}");
    }

    #[test]
    fn programme_absent() {
        assert!(run_with_timeout("outil-qui-nexiste-pas", &["--help"], Duration::from_secs(1)).is_none());
    }

    #[test]
    fn trouve_les_readmes_et_ignore_les_dossiers_de_build() {
        let dir = std::env::temp_dir().join("mnemors-test-readmes");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(dir.join("sous")).unwrap();
        std::fs::write(dir.join("README.md"), "# Installer\n\nnpm install").unwrap();
        std::fs::write(dir.join("sous/notes.md"), "notes").unwrap();
        std::fs::write(dir.join("node_modules/pkg/README.md"), "ignoré").unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}").unwrap();

        let docs = readmes(&dir);
        let sources: Vec<String> = docs.iter().map(|d| d.source.clone()).collect();
        assert_eq!(docs.len(), 2, "sources : {sources:?}");
        assert!(sources.iter().any(|s| s.ends_with("README.md")));
        assert!(sources.iter().any(|s| s.ends_with("notes.md")));
        assert!(!sources.iter().any(|s| s.contains("node_modules")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn outils_interdits_ignores() {
        let docs = collect(&["sudo".to_string(), "vim".to_string()], Path::new("/tmp"));
        assert!(docs.is_empty());
    }
}
