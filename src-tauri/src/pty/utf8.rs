/// Découpe un flux d'octets en chaînes UTF-8 valides, même quand une
/// séquence est coupée entre deux lectures du PTY.
#[derive(Default)]
pub struct Utf8Chunker {
    carry: Vec<u8>,
}

impl Utf8Chunker {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.carry.extend_from_slice(bytes);
        let mut text = String::new();

        loop {
            match std::str::from_utf8(&self.carry) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.carry.clear();
                    break;
                }
                Err(err) => {
                    let valid = err.valid_up_to();
                    text.push_str(std::str::from_utf8(&self.carry[..valid]).unwrap());
                    match err.error_len() {
                        // Octets invalides : on les remplace et on continue.
                        Some(len) => {
                            text.push('\u{FFFD}');
                            self.carry.drain(..valid + len);
                            if self.carry.is_empty() {
                                break;
                            }
                        }
                        // Séquence incomplète : on la garde pour la prochaine lecture.
                        None => {
                            self.carry.drain(..valid);
                            break;
                        }
                    }
                }
            }
        }

        text
    }

    pub fn flush(&mut self) -> String {
        if self.carry.is_empty() {
            return String::new();
        }
        let text = String::from_utf8_lossy(&self.carry).into_owned();
        self.carry.clear();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8Chunker;

    #[test]
    fn sequence_coupee_entre_deux_lectures() {
        let mut chunker = Utf8Chunker::default();
        assert_eq!(chunker.push(&"hé".as_bytes()[..2]), "h");
        assert_eq!(chunker.push(&"hé".as_bytes()[2..]), "é");
    }

    #[test]
    fn emoji_coupe_sur_quatre_octets() {
        let mut chunker = Utf8Chunker::default();
        let emoji = "🚀".as_bytes();
        assert_eq!(chunker.push(&emoji[..2]), "");
        assert_eq!(chunker.push(&emoji[2..]), "🚀");
    }

    #[test]
    fn octet_invalide_est_remplace() {
        let mut chunker = Utf8Chunker::default();
        assert_eq!(chunker.push(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
        assert_eq!(chunker.push(b"ok"), "ok");
    }

    #[test]
    fn flush_rend_une_sequence_incomplete() {
        let mut chunker = Utf8Chunker::default();
        assert_eq!(chunker.push(&[0xE2, 0x82]), "");
        assert_eq!(chunker.flush(), "\u{FFFD}");
    }
}
