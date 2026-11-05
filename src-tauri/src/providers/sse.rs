/// Extrait les événements `data:` complets d'un tampon SSE.
/// Travaille sur des octets : une séquence UTF-8 peut être coupée entre deux
/// lectures réseau, on ne décode donc qu'une fois l'événement complet.
pub fn take_events(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut events = Vec::new();

    while let Some(end) = event_end(buffer) {
        let block: Vec<u8> = buffer.drain(..end).collect();
        let text = String::from_utf8_lossy(&block);
        let payload = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|data| data.trim_start())
            .collect::<Vec<_>>()
            .join("\n");
        if !payload.is_empty() {
            events.push(payload);
        }
    }

    events
}

/// Fin de l'événement : première ligne vide, quel que soit le style de fin
/// de ligne (`\n\n`, `\r\n\r\n`, `\r\r`).
fn event_end(buffer: &[u8]) -> Option<usize> {
    for i in 0..buffer.len() {
        if buffer[i] == b'\n' {
            if buffer.get(i + 1) == Some(&b'\n') {
                return Some(i + 2);
            }
            if buffer.get(i + 1) == Some(&b'\r') && buffer.get(i + 2) == Some(&b'\n') {
                return Some(i + 3);
            }
        } else if buffer[i] == b'\r' {
            if buffer.get(i + 1) == Some(&b'\r') {
                return Some(i + 2);
            }
            if buffer.get(i + 1) == Some(&b'\n') && buffer.get(i + 2) == Some(&b'\n') {
                return Some(i + 3);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(input: &str) -> (Vec<String>, String) {
        let mut buffer = input.as_bytes().to_vec();
        let events = take_events(&mut buffer);
        (events, String::from_utf8(buffer).unwrap())
    }

    #[test]
    fn evenement_simple() {
        let (events, rest) = drain("data: {\"a\":1}\n\n");
        assert_eq!(events, vec!["{\"a\":1}"]);
        assert!(rest.is_empty());
    }

    #[test]
    fn plusieurs_evenements_et_reste_incomplet() {
        let (events, rest) = drain("data: un\n\ndata: deux\n\ndata: tro");
        assert_eq!(events, vec!["un", "deux"]);
        assert_eq!(rest, "data: tro");
    }

    #[test]
    fn crlf_accepte() {
        let (events, rest) = drain("data: un\r\n\r\ndata: deux\r\n");
        assert_eq!(events, vec!["un"]);
        assert_eq!(rest, "data: deux\r\n");
    }

    #[test]
    fn commentaires_ignores() {
        let (events, _) = drain(": keep-alive\n\ndata: ok\n\n");
        assert_eq!(events, vec!["ok"]);
    }

    #[test]
    fn data_multiligne_recolle() {
        let (events, _) = drain("data: a\ndata: b\n\n");
        assert_eq!(events, vec!["a\nb"]);
    }

    #[test]
    fn utf8_coupe_entre_deux_lectures() {
        let payload = "data: héllo\n\n";
        let bytes = payload.as_bytes();
        let cut = payload.find('é').unwrap() + 1;

        let mut buffer = bytes[..cut].to_vec();
        assert!(take_events(&mut buffer).is_empty());

        buffer.extend_from_slice(&bytes[cut..]);
        assert_eq!(take_events(&mut buffer), vec!["héllo"]);
    }
}
