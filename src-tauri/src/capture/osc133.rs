use std::fmt;

const ESC: char = '\x1b';
const BEL: char = '\x07';

#[derive(Debug, PartialEq)]
pub enum Captured {
    Prompt,
    Command(String),
    OutputStart,
    Done(i32),
    Cwd(String),
}

#[derive(Debug, PartialEq)]
pub enum Piece {
    Text(String),
    Event(Captured),
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Ground,
    Escape,
    Osc,
    OscEscape,
}

impl fmt::Display for Captured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Captured::Prompt => write!(f, "prompt"),
            Captured::Command(_) => write!(f, "commande"),
            Captured::OutputStart => write!(f, "sortie"),
            Captured::Done(code) => write!(f, "fin ({code})"),
            Captured::Cwd(path) => write!(f, "cwd {path}"),
        }
    }
}

/// Parseur incrémental des séquences OSC 133 / OSC 7 émises par le shell.
/// Il ne consomme que les séquences qu'il comprend et laisse passer le reste.
pub struct Osc133Parser {
    state: State,
    osc: String,
}

impl Default for Osc133Parser {
    fn default() -> Self {
        Self {
            state: State::Ground,
            osc: String::new(),
        }
    }
}

impl Osc133Parser {
    pub fn feed(&mut self, input: &str) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut text = String::new();

        for c in input.chars() {
            match self.state {
                State::Ground => {
                    if c == ESC {
                        flush_text(&mut text, &mut pieces);
                        self.state = State::Escape;
                    } else {
                        text.push(c);
                    }
                }
                State::Escape => {
                    if c == ']' {
                        self.osc.clear();
                        self.state = State::Osc;
                    } else {
                        text.push(ESC);
                        text.push(c);
                        self.state = State::Ground;
                    }
                }
                State::Osc => {
                    if c == BEL {
                        self.dispatch(&mut pieces);
                        self.state = State::Ground;
                    } else if c == ESC {
                        self.state = State::OscEscape;
                    } else {
                        self.osc.push(c);
                    }
                }
                State::OscEscape => {
                    if c == '\\' {
                        self.dispatch(&mut pieces);
                        self.state = State::Ground;
                    } else {
                        self.osc.push(ESC);
                        self.osc.push(c);
                        self.state = State::Osc;
                    }
                }
            }
        }

        flush_text(&mut text, &mut pieces);
        pieces
    }

    fn dispatch(&mut self, pieces: &mut Vec<Piece>) {
        let content = std::mem::take(&mut self.osc);
        let (code, payload) = content.split_once(';').unwrap_or((&content, ""));

        match code {
            "133" => {
                let (kind, rest) = payload.split_once(';').unwrap_or((payload, ""));
                match kind {
                    "A" => pieces.push(Piece::Event(Captured::Prompt)),
                    "B" => {
                        if let Some(cmd) = hex_decode(rest).filter(|c| !c.is_empty()) {
                            pieces.push(Piece::Event(Captured::Command(cmd)));
                        }
                    }
                    "C" => pieces.push(Piece::Event(Captured::OutputStart)),
                    "D" => pieces.push(Piece::Event(Captured::Done(
                        rest.trim().parse().unwrap_or(-1),
                    ))),
                    _ => {}
                }
            }
            "7" => {
                if let Some(path) = osc7_path(payload) {
                    pieces.push(Piece::Event(Captured::Cwd(path)));
                }
            }
            // Titre de fenêtre, progression, etc. : on les rend tels quels à xterm.
            _ => pieces.push(Piece::Text(format!("\x1b]{content}\x07"))),
        }
    }
}

fn flush_text(text: &mut String, pieces: &mut Vec<Piece>) {
    if !text.is_empty() {
        pieces.push(Piece::Text(std::mem::take(text)));
    }
}

fn hex_decode(input: &str) -> Option<String> {
    if input.len() % 2 != 0 || !input.is_ascii() {
        return None;
    }
    let bytes = (0..input.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn osc7_path(payload: &str) -> Option<String> {
    let rest = payload.strip_prefix("file://")?;
    let path_start = rest.find('/')?;
    Some(percent_decode(&rest[path_start..]))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            if let Ok(value) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(chunks: &[&str]) -> Vec<Piece> {
        let mut parser = Osc133Parser::default();
        chunks.iter().flat_map(|c| parser.feed(c)).collect()
    }

    #[test]
    fn texte_sans_sequence_est_intact() {
        assert_eq!(
            feed_all(&["hello", " world"]),
            vec![Piece::Text("hello".into()), Piece::Text(" world".into())]
        );
    }

    #[test]
    fn marqueurs_simples() {
        let pieces = feed_all(&["\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;0\x07"]);
        assert_eq!(
            pieces,
            vec![
                Piece::Event(Captured::Prompt),
                Piece::Event(Captured::OutputStart),
                Piece::Event(Captured::Done(0)),
            ]
        );
    }

    #[test]
    fn sequence_coupee_entre_deux_lectures() {
        let cmd = hex_encode("git status");
        let pieces = feed_all(&[
            "\x1b]13",
            "3;B;",
            &cmd[..4],
            &cmd[4..],
            "\x07",
            "sortie",
        ]);
        assert_eq!(
            pieces,
            vec![
                Piece::Event(Captured::Command("git status".into())),
                Piece::Text("sortie".into()),
            ]
        );
    }

    #[test]
    fn terminateur_st_accepte() {
        let pieces = feed_all(&["\x1b]133;A\x1b\\"]);
        assert_eq!(pieces, vec![Piece::Event(Captured::Prompt)]);
    }

    #[test]
    fn commande_multiligne_et_utf8() {
        let cmd = hex_encode("echo 'hé\nligne 2'");
        let pieces = feed_all(&[&format!("\x1b]133;B;{cmd}\x07")]);
        assert_eq!(
            pieces,
            vec![Piece::Event(Captured::Command(
                "echo 'hé\nligne 2'".into()
            ))]
        );
    }

    #[test]
    fn commande_vide_ignoree() {
        assert_eq!(feed_all(&["\x1b]133;B;\x07"]), vec![]);
    }

    #[test]
    fn cwd_percent_decode() {
        let pieces = feed_all(&["\x1b]7;file://localhost/home/moi/mes%20docs\x07"]);
        assert_eq!(
            pieces,
            vec![Piece::Event(Captured::Cwd("/home/moi/mes docs".into()))]
        );
    }

    #[test]
    fn autres_osc_preserves() {
        let pieces = feed_all(&["\x1b]0;mon titre\x07"]);
        assert_eq!(pieces, vec![Piece::Text("\x1b]0;mon titre\x07".into())]);
    }

    #[test]
    fn esc_non_osc_preserve() {
        let pieces = feed_all(&["\x1b[31mrouge\x1b[0m"]);
        let text: String = pieces
            .into_iter()
            .map(|piece| match piece {
                Piece::Text(text) => text,
                other => panic!("événement inattendu : {other:?}"),
            })
            .collect();
        assert_eq!(text, "\x1b[31mrouge\x1b[0m");
    }

    fn hex_encode(input: &str) -> String {
        input.bytes().map(|b| format!("{b:02x}")).collect()
    }
}
