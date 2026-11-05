use serde::Deserialize;

/// Commande passée telle qu'injectée dans le contexte du modèle.
#[derive(Debug, Clone, Deserialize)]
pub struct CommandContext {
    pub cmd: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub output: String,
}

pub struct ContextInput<'a> {
    pub cwd: Option<&'a str>,
    pub shell: &'a str,
    pub recent: Vec<CommandContext>,
    pub failures: Vec<CommandContext>,
}

/// Budget approximatif en caractères (aucun tokenizer : ~4 caractères/token).
const BUDGET_CHARS: usize = 24_000;
const OUTPUT_CHARS: usize = 1_500;
const FAILURE_OUTPUT_CHARS: usize = 800;

const SYSTEM: &str = "Tu es l'assistant intégré à MnemoRS, un terminal. \
Tu aides l'utilisateur à comprendre et corriger les commandes qu'il exécute. \
Le contexte ci-dessous provient de son historique réel : commandes, dossier, \
code de retour et sortie. Appuie-toi dessus et cite les éléments précis \
(commande, code de retour, message d'erreur). N'invente jamais un code de \
retour, une commande ou une sortie absents du contexte ; si l'information \
manque, dis-le. Réponds uniquement en français, sans mélanger d'anglais, de façon \
concise, et propose des commandes en bloc de code quand c'est utile.";

/// Assemble le prompt système : en-tête, commandes récentes puis échecs,
/// en supprimant les éléments les plus anciens quand le budget est dépassé.
pub fn build(input: &ContextInput) -> String {
    let mut out = String::from(SYSTEM);

    out.push_str("\n\n## Session");
    out.push_str(&format!(
        "\n- Dossier courant : {}",
        input.cwd.unwrap_or("inconnu")
    ));
    out.push_str(&format!("\n- Shell : {}", input.shell));

    if input.recent.is_empty() {
        out.push_str("\n\nAucune commande enregistrée dans cette session.");
        return out;
    }

    out.push_str("\n\n## Commandes récentes");
    out.push_str("\nDe la plus ancienne à la plus récente.\n");

    let mut used = out.len();
    let mut selected: Vec<&CommandContext> = Vec::new();
    for command in input.recent.iter().rev() {
        let cost = estimate(command, OUTPUT_CHARS);
        if used + cost > BUDGET_CHARS {
            continue;
        }
        used += cost;
        selected.push(command);
    }
    selected.reverse();

    if selected.is_empty() {
        out.push_str("\n(historique trop volumineux pour le budget)\n");
    }
    for command in selected {
        out.push_str(&format_command(command, OUTPUT_CHARS));
    }

    if !input.failures.is_empty() {
        out.push_str("\n## Échecs récents");
        out.push_str("\nCommandes qui ont échoué, pour repérer les erreurs récurrentes.\n");
        for command in &input.failures {
            let cost = estimate(command, FAILURE_OUTPUT_CHARS);
            if used + cost > BUDGET_CHARS {
                break;
            }
            used += cost;
            out.push_str(&format_command(command, FAILURE_OUTPUT_CHARS));
        }
    }

    out
}

fn estimate(command: &CommandContext, output_chars: usize) -> usize {
    command.cmd.len() + command.output.len().min(output_chars) + 80
}

fn format_command(command: &CommandContext, output_chars: usize) -> String {
    let mut out = String::from("\n```\n$ ");
    out.push_str(command.cmd.trim());
    out.push_str("\n```\n");

    let mut meta = Vec::new();
    if let Some(cwd) = &command.cwd {
        meta.push(format!("dossier {cwd}"));
    }
    if let Some(code) = command.exit_code {
        meta.push(format!("code {code}"));
    }
    if !meta.is_empty() {
        out.push_str(&format!("({})\n", meta.join(", ")));
    }

    let output = command.output.trim();
    if output.is_empty() {
        out.push_str("(aucune sortie)\n");
    } else {
        out.push_str("```\n");
        out.push_str(&truncate(output, output_chars));
        out.push_str("\n```\n");
    }
    out
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let head: String = input.chars().take(max * 2 / 3).collect();
    let tail: String = input
        .chars()
        .skip(input.chars().count() - max / 3)
        .collect();
    format!("{head}\n…[tronqué]…\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(cmd: &str, output: &str, exit_code: i32) -> CommandContext {
        CommandContext {
            cmd: cmd.into(),
            cwd: Some("/home/moi".into()),
            exit_code: Some(exit_code),
            output: output.into(),
        }
    }

    fn input(recent: Vec<CommandContext>, failures: Vec<CommandContext>) -> ContextInput<'static> {
        ContextInput {
            cwd: Some("/home/moi/projet"),
            shell: "zsh",
            recent,
            failures,
        }
    }

    #[test]
    fn contient_dossier_shell_et_commande() {
        let prompt = build(&input(vec![command("ls", "fichier", 0)], vec![]));
        assert!(prompt.contains("/home/moi/projet"));
        assert!(prompt.contains("zsh"));
        assert!(prompt.contains("$ ls"));
        assert!(prompt.contains("code 0"));
        assert!(prompt.contains("fichier"));
    }

    #[test]
    fn historique_vide_reste_valide() {
        let prompt = build(&input(vec![], vec![]));
        assert!(prompt.contains("Aucune commande"));
        assert!(prompt.contains("N'invente jamais"));
    }

    #[test]
    fn ordre_chronologique_conserve() {
        let prompt = build(&input(
            vec![
                command("premiere", "", 0),
                command("deuxieme", "", 0),
                command("troisieme", "", 0),
            ],
            vec![],
        ));
        let a = prompt.find("premiere").unwrap();
        let b = prompt.find("deuxieme").unwrap();
        let c = prompt.find("troisieme").unwrap();
        assert!(a < b && b < c);
    }

    #[test]
    fn sortie_longue_tronquee() {
        let long = "x".repeat(10_000);
        let prompt = build(&input(vec![command("cat gros", &long, 0)], vec![]));
        assert!(prompt.contains("[tronqué]"));
        assert!(prompt.len() < 10_000);
    }

    #[test]
    fn budget_respecte_en_gardant_les_plus_recentes() {
        let mut recent = Vec::new();
        for i in 0..500 {
            recent.push(command(&format!("commande-{i}"), &"y".repeat(1000), 0));
        }
        let prompt = build(&input(recent, vec![]));
        assert!(prompt.len() <= BUDGET_CHARS + 2000, "taille {}", prompt.len());
        assert!(prompt.contains("commande-499"));
        assert!(!prompt.contains("commande-0"));
    }

    #[test]
    fn echecs_inclus_et_tronques_plus_court() {
        let prompt = build(&input(
            vec![command("true", "", 0)],
            vec![command("false", &"z".repeat(5000), 1)],
        ));
        assert!(prompt.contains("Échecs récents"));
        assert!(prompt.contains("$ false"));
        assert!(prompt.contains("code 1"));
    }

    #[test]
    fn sortie_vide_est_signalee() {
        let prompt = build(&input(vec![command("false", "", 1)], vec![]));
        assert!(prompt.contains("(aucune sortie)"));
    }
}
