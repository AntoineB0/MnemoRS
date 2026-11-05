use std::path::Path;

use super::ShellIntegration;

pub struct Zsh;

impl ShellIntegration for Zsh {
    fn id(&self) -> &'static str {
        "zsh"
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[
            (
                ".zshenv",
                include_str!("../../shell-integration/zsh/.zshenv"),
            ),
            (".zshrc", include_str!("../../shell-integration/zsh/.zshrc")),
        ]
    }

    fn env(&self, injection_dir: &Path) -> Vec<(String, String)> {
        let mut vars = vec![("ZDOTDIR".to_string(), injection_dir.display().to_string())];
        if let Ok(existing) = std::env::var("ZDOTDIR") {
            if !existing.is_empty() {
                vars.push(("MNEMORS_ORIGINAL_ZDOTDIR".to_string(), existing));
            }
        }
        vars
    }
}
