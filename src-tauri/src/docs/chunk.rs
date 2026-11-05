#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub section: Option<String>,
    pub content: String,
}

/// Découpe un texte en chunks en suivant les titres de section.
/// Les lignes d'un paragraphe sont recolllées (les pages man sont coupées à
/// 80 colonnes), sauf dans les blocs de code où les retours sont significatifs.
pub fn chunk(text: &str, max_chars: usize) -> Vec<Chunk> {
    let max_chars = max_chars.max(200);
    let mut chunks = Vec::new();
    let mut section: Option<String> = None;
    let mut paragraph: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            paragraph.push(line.to_string());
            continue;
        }

        if !in_fence && is_man_header(trimmed) {
            continue;
        }

        if !in_fence {
            if let Some(title) = heading(trimmed, lines.peek().map(|next| next.trim())) {
                flush(&mut chunks, &mut paragraph, &section, max_chars);
                section = Some(title);
                continue;
            }
        }

        if trimmed.is_empty() && !in_fence {
            flush(&mut chunks, &mut paragraph, &section, max_chars);
            continue;
        }

        paragraph.push(line.to_string());
    }

    flush(&mut chunks, &mut paragraph, &section, max_chars);
    chunks
}

/// En-tête de page man (`TAR(1) ... TAR(1)`) : présent en haut et en bas,
/// sans valeur pour la recherche.
fn is_man_header(line: &str) -> bool {
    header_at_start(line) || header_at_end(line)
}

fn header_at_start(line: &str) -> bool {
    let Some(open) = line.find('(') else {
        return false;
    };
    is_man_name(&line[..open]) && line[open + 1..].starts_with(|c: char| c.is_ascii_digit())
}

/// Pied de page : `TAR            11 juillet 2022            TAR(1)`.
fn header_at_end(line: &str) -> bool {
    let Some(close) = line.rfind(')') else {
        return false;
    };
    if close + 1 != line.len() {
        return false;
    }
    let Some(open) = line[..close].rfind('(') else {
        return false;
    };
    let name = line[..open].rsplit(' ').next().unwrap_or("");
    is_man_name(name) && line[open + 1..close].starts_with(|c: char| c.is_ascii_digit())
}

fn is_man_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_uppercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// Titre markdown (`## X`) ou titre de page man (ligne en majuscules).
fn heading(line: &str, next: Option<&str>) -> Option<String> {
    if let Some(rest) = line.strip_prefix('#') {
        let title = rest.trim_start_matches('#').trim();
        if !title.is_empty() {
            return Some(title.to_string());
        }
    }

    let candidate = line.trim_end_matches(':');
    let len = candidate.chars().count();
    if !(2..=60).contains(&len) {
        return None;
    }
    let upper = candidate
        .chars()
        .all(|c| c.is_uppercase() || c.is_ascii_digit() || matches!(c, ' ' | '-' | '_' | '\'' | '/'));
    if !upper || !candidate.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    // Un titre est isolé : suivi d'une ligne vide ou du début d'un paragraphe.
    let isolated = next.is_none() || next.is_some_and(|n| n.is_empty() || !n.starts_with(char::is_whitespace));
    isolated.then(|| candidate.to_string())
}

fn flush(chunks: &mut Vec<Chunk>, paragraph: &mut Vec<String>, section: &Option<String>, max_chars: usize) {
    if paragraph.is_empty() {
        return;
    }

    let preserve = paragraph.iter().any(|line| line.trim_start().starts_with("```"));
    let joined = if preserve {
        paragraph.join("\n")
    } else {
        paragraph
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" ")
    };
    paragraph.clear();

    let joined = joined.trim().to_string();
    if joined.is_empty() {
        return;
    }

    for piece in split_max(&joined, max_chars) {
        chunks.push(Chunk {
            section: section.clone(),
            content: piece,
        });
    }
}

/// Coupe un texte trop long aux limites de mots.
fn split_max(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }

    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in text.split_inclusive(char::is_whitespace) {
        if current.chars().count() + word.chars().count() > max_chars && !current.is_empty() {
            pieces.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(word);
    }
    if !current.trim().is_empty() {
        pieces.push(current.trim().to_string());
    }
    pieces
}

/// Retire les overstrikes (`X\bX`) et les séquences ANSI d'une sortie man/help.
pub fn clean_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x08' {
            out.pop();
            continue;
        }
        if c == '\x1b' {
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
            continue;
        }
        if c == '\r' {
            continue;
        }
        out.push(c);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_man() {
        let text = "TAR(1)\n\nNOM\n       tar - Utilitaire d'archivage\n\nSYNOPSIS\n       tar -xzf archive\n";
        let chunks = chunk(text, 1200);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].section.as_deref(), Some("NOM"));
        assert!(chunks[0].content.contains("Utilitaire d'archivage"));
        assert_eq!(chunks[1].section.as_deref(), Some("SYNOPSIS"));
    }

    #[test]
    fn sections_markdown() {
        let text = "# Installation\n\nCloner le dépôt.\n\n## Lancement\n\nLancer npm run tauri dev.\n";
        let chunks = chunk(text, 1200);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].section.as_deref(), Some("Installation"));
        assert_eq!(chunks[1].section.as_deref(), Some("Lancement"));
    }

    #[test]
    fn lignes_recollees_hors_code() {
        let text = "NOM\n       une phrase\n       qui continue ici\n";
        let chunks = chunk(text, 1200);
        assert_eq!(chunks[0].content, "une phrase qui continue ici");
    }

    #[test]
    fn bloc_de_code_preserve() {
        let text = "## Commandes\n\n```\nnpm install\nnpm run tauri dev\n```\n";
        let chunks = chunk(text, 1200);
        assert!(chunks[0].content.contains("npm install\nnpm run tauri dev"));
    }

    #[test]
    fn section_longue_decoupee() {
        let body = "mot ".repeat(600);
        let text = format!("## Options\n\n{body}");
        let chunks = chunk(&text, 500);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.content.chars().count() <= 500));
        assert!(chunks.iter().all(|c| c.section.as_deref() == Some("Options")));
    }

    #[test]
    fn texte_vide() {
        assert!(chunk("", 1200).is_empty());
        assert!(chunk("\n\n\n", 1200).is_empty());
    }

    #[test]
    #[ignore = "dépend de la page man locale"]
    fn page_man_reelle_decoupee() {
        let output = std::process::Command::new("man")
            .args(["-P", "cat", "tar"])
            .output()
            .expect("man tar");
        let text = clean_text(&String::from_utf8_lossy(&output.stdout));
        let chunks = chunk(&text, 1200);
        assert!(chunks.len() > 5, "chunks : {}", chunks.len());
        assert!(chunks.iter().any(|c| c.section.as_deref() == Some("OPTIONS")));
        assert!(chunks.iter().all(|c| !c.content.contains("TAR(1)")));
    }

    #[test]
    fn overstrikes_et_ansi_nettoyes() {
        assert_eq!(clean_text("g\x08gros"), "gros");
        assert_eq!(clean_text("\x1b[1mtexte\x1b[0m"), "texte");
        assert_eq!(clean_text("a\r\nb"), "a\nb");
    }
}
