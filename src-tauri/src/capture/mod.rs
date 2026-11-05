mod osc133;

use std::sync::LazyLock;
use std::time::Instant;

use regex::Regex;

use osc133::{Captured, Osc133Parser, Piece};

/// Plafond du buffer de sortie gardé en mémoire pendant l'exécution.
const MAX_BUFFER: usize = 256 * 1024;
const OUTPUT_HEAD: usize = 2 * 1024;
const OUTPUT_TAIL: usize = 30 * 1024;
const COMMAND_MAX: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct CommandRecord {
    pub cmd: String,
    pub cwd: Option<String>,
    pub exit_code: i32,
    pub output: String,
    pub truncated: bool,
    pub sensitive: bool,
    pub duration_ms: u64,
}

pub struct Capture {
    parser: Osc133Parser,
    cwd: Option<String>,
    pending: Option<String>,
    output: String,
    output_truncated: bool,
    capturing_output: bool,
    started: Option<Instant>,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            parser: Osc133Parser::default(),
            cwd: None,
            pending: None,
            output: String::new(),
            output_truncated: false,
            capturing_output: false,
            started: None,
        }
    }
}

impl Capture {
    /// Consomme un morceau de sortie PTY et renvoie le texte à afficher,
    /// plus les commandes qui viennent de se terminer.
    pub fn feed(&mut self, input: &str) -> (String, Vec<CommandRecord>) {
        let mut visible = String::with_capacity(input.len());
        let mut records = Vec::new();

        for piece in self.parser.feed(input) {
            match piece {
                Piece::Text(text) => {
                    if self.capturing_output {
                        self.push_output(&text);
                    }
                    visible.push_str(&text);
                }
                Piece::Event(event) => self.handle(event, &mut records),
            }
        }

        (visible, records)
    }

    fn handle(&mut self, event: Captured, records: &mut Vec<CommandRecord>) {
        match event {
            Captured::Prompt => {}
            Captured::Command(cmd) => {
                self.pending = Some(cmd);
                self.output.clear();
                self.output_truncated = false;
                self.capturing_output = false;
                self.started = None;
            }
            Captured::OutputStart => {
                self.capturing_output = true;
                self.started = Some(Instant::now());
            }
            Captured::Done(code) => {
                let Some(cmd) = self.pending.take() else {
                    return;
                };
                let duration_ms = self
                    .started
                    .take()
                    .map(|started| started.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                let sensitive = is_sensitive(&cmd) || has_secret_prompt(&self.output);

                let cleaned = strip_ansi(&self.output);
                let (output, cut) = truncate(&cleaned, OUTPUT_HEAD, OUTPUT_TAIL);
                let output = mask_secrets(&output);
                let cmd = mask_secrets(&truncate_chars(&cmd, COMMAND_MAX));

                records.push(CommandRecord {
                    cmd,
                    cwd: self.cwd.clone(),
                    exit_code: code,
                    output: if sensitive { String::new() } else { output },
                    truncated: self.output_truncated || cut,
                    sensitive,
                    duration_ms,
                });

                self.output.clear();
                self.output_truncated = false;
                self.capturing_output = false;
            }
            Captured::Cwd(path) => self.cwd = Some(path),
        }
    }

    fn push_output(&mut self, text: &str) {
        self.output.push_str(text);
        if self.output.len() > MAX_BUFFER {
            let head = safe_prefix(&self.output, OUTPUT_HEAD).to_string();
            let tail = safe_suffix(&self.output, MAX_BUFFER - OUTPUT_HEAD);
            self.output = format!("{head}\n…[tronqué]…\n{tail}");
            self.output_truncated = true;
        }
    }
}

fn is_sensitive(cmd: &str) -> bool {
    let first = cmd.split_whitespace().next().unwrap_or("");
    let name = first.rsplit('/').next().unwrap_or(first);
    matches!(
        name,
        "sudo" | "su" | "ssh" | "scp" | "pass" | "gpg" | "op" | "vault"
    )
}

fn has_secret_prompt(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("password:")
        || lower.contains("passphrase")
        || lower.contains("[sudo]")
}

static SECRET_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(password|passwd|passphrase|token|secret|api[_-]?key)\s*[:=]\s*\S+")
        .expect("regex valide")
});

static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{12,}").expect("regex valide")
});

static API_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-[A-Za-z0-9-]{10,})")
        .expect("regex valide")
});

fn mask_secrets(input: &str) -> String {
    let out = SECRET_ASSIGN.replace_all(input, "$1: [masqué]");
    let out = BEARER.replace_all(&out, "Bearer [masqué]");
    API_KEY.replace_all(&out, "[masqué]").into_owned()
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if c != '\r' {
                out.push(c);
            }
            continue;
        }

        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    out
}

fn truncate(input: &str, head: usize, tail: usize) -> (String, bool) {
    if input.len() <= head + tail {
        return (input.to_string(), false);
    }
    (
        format!(
            "{}\n…[tronqué]…\n{}",
            safe_prefix(input, head),
            safe_suffix(input, tail)
        ),
        true,
    )
}

fn safe_prefix(input: &str, max: usize) -> &str {
    let mut end = max.min(input.len());
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

fn safe_suffix(input: &str, max: usize) -> &str {
    let mut start = input.len().saturating_sub(max);
    while !input.is_char_boundary(start) {
        start += 1;
    }
    &input[start..]
}

fn truncate_chars(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        format!("{}…", input.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(input: &str) -> String {
        input.bytes().map(|b| format!("{b:02x}")).collect()
    }

    fn command(cmd: &str) -> String {
        format!("\x1b]133;B;{}\x07\x1b]133;C\x07", hex(cmd))
    }

    fn done(code: i32) -> String {
        format!("\x1b]133;D;{code}\x07\x1b]133;A\x07")
    }

    #[test]
    fn cycle_complet() {
        let mut capture = Capture::default();
        let mut records = Vec::new();

        let (visible, r) = capture.feed(&format!(
            "{}hello\n{}",
            command("echo hello"),
            done(0)
        ));
        records.extend(r);

        assert_eq!(visible, "hello\n");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.cmd, "echo hello");
        assert_eq!(record.exit_code, 0);
        assert_eq!(record.output, "hello\n");
        assert!(!record.truncated);
        assert!(!record.sensitive);
    }

    #[test]
    fn prompt_hors_sortie() {
        let mut capture = Capture::default();
        let (prompt_visible, _) = capture.feed("\x1b]133;A\x07$ ");
        assert_eq!(prompt_visible, "$ ");

        capture.feed(&command("echo hi"));
        capture.feed("hi\n");
        let (_, records) = capture.feed(&done(0));
        assert_eq!(records[0].output, "hi\n");
    }

    #[test]
    fn code_retour_et_cwd() {
        let mut capture = Capture::default();
        capture.feed("\x1b]7;file://host/home/moi/projet\x07");
        capture.feed(&command("false"));
        let (_, records) = capture.feed(&done(1));

        assert_eq!(records[0].exit_code, 1);
        assert_eq!(records[0].cwd.as_deref(), Some("/home/moi/projet"));
    }

    #[test]
    fn premier_prompt_ignore() {
        let mut capture = Capture::default();
        let (_, records) = capture.feed(&done(0));
        assert!(records.is_empty());
    }

    #[test]
    fn ansi_strippe() {
        assert_eq!(strip_ansi("\x1b[31mrouge\x1b[0m ok"), "rouge ok");
        assert_eq!(strip_ansi("a\r\nb"), "a\nb");
        assert_eq!(strip_ansi("\x1b]0;titre\x07texte"), "texte");
    }

    #[test]
    fn secrets_masques() {
        assert_eq!(
            mask_secrets("export API_KEY=deadbeef1234"),
            "export API_KEY: [masqué]"
        );
        assert_eq!(
            mask_secrets("curl -H 'Authorization: Bearer abcdefghijklmnop'"),
            "curl -H 'Authorization: Bearer [masqué]'"
        );
        assert_eq!(
            mask_secrets("token sk-abcdefghijklmnopqrst"),
            "token [masqué]"
        );
    }

    #[test]
    fn commande_sensible_sans_sortie() {
        let mut capture = Capture::default();
        capture.feed(&command("sudo cat /etc/shadow"));
        let (_, records) = capture.feed("root:hash\n");
        let (_, records2) = capture.feed(&done(0));

        assert_eq!(records.len(), 0);
        assert_eq!(records2.len(), 1);
        assert!(records2[0].sensitive);
        assert_eq!(records2[0].output, "");
    }

    #[test]
    fn grosse_sortie_tronquee_avec_entete_et_queue() {
        let mut capture = Capture::default();
        capture.feed(&command("seq 100000"));
        let big = "x".repeat(MAX_BUFFER * 2);
        capture.feed(&big);
        let (_, records) = capture.feed(&done(0));

        let record = &records[0];
        assert!(record.truncated);
        assert!(record.output.len() < MAX_BUFFER);
        assert!(record.output.contains("[tronqué]"));
    }
}
