use std::path::Path;

/// Intégration shell chargée d'émettre les marqueurs OSC 133.
/// Ajouter un shell = ajouter un module et une ligne dans `detect`.
pub trait ShellIntegration: Send + Sync {
    fn id(&self) -> &'static str;

    /// Fichiers de configuration à écrire dans le dossier d'injection.
    fn files(&self) -> &'static [(&'static str, &'static str)];

    /// Variables d'environnement à injecter au lancement du shell.
    fn env(&self, injection_dir: &Path) -> Vec<(String, String)>;

    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn install(&self, injection_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(injection_dir)?;
        for (name, content) in self.files() {
            std::fs::write(injection_dir.join(name), content)?;
        }
        Ok(())
    }
}

mod zsh;

pub fn detect(shell: &str) -> Option<Box<dyn ShellIntegration>> {
    match Path::new(shell).file_name()?.to_str()? {
        "zsh" => Some(Box::new(zsh::Zsh)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_est_detecte() {
        assert_eq!(detect("/usr/bin/zsh").unwrap().id(), "zsh");
        assert!(detect("/usr/bin/bash").is_none());
    }

    #[test]
    fn zsh_installe_ses_fichiers_et_redirige_zdotdir() {
        let dir = std::env::temp_dir().join("mnemors-test-shell");
        let shell = detect("zsh").unwrap();
        shell.install(&dir).unwrap();

        assert!(dir.join(".zshrc").is_file());
        assert!(dir.join(".zshenv").is_file());

        let env = shell.env(&dir);
        assert!(env.contains(&("ZDOTDIR".to_string(), dir.display().to_string())));

        std::fs::remove_dir_all(&dir).ok();
    }
}
