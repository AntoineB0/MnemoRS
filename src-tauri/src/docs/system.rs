/// Empreinte de la base de paquets du système : elle change quand un paquet
/// est installé, mis à jour ou supprimé, ce qui invalide les pages man.
const CANDIDATES: &[&str] = &[
    "/usr/lib/sysimage/rpm/rpmdb.sqlite",
    "/var/lib/rpm/rpmdb.sqlite",
    "/var/lib/dpkg/status",
    "/var/lib/pacman/local",
];

pub fn fingerprint() -> Option<String> {
    CANDIDATES.iter().find_map(|path| {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(format!("{path}:{modified}:{}", meta.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empreinte_stable_et_non_vide() {
        let first = fingerprint();
        let second = fingerprint();
        assert_eq!(first, second);
        if let Some(value) = first {
            assert!(value.contains(':'));
        }
    }
}
