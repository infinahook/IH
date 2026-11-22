//! Adaptateurs d'agents de développement.
//!
//! Chaque CLI a sa propre façon de recevoir une instruction unique. IH ne
//! devine rien : la commande est construite explicitement et peut être
//! surchargée par la configuration (`args` contenant `{prompt}`).
//!
//! Limite assumée : ces contrats d'appel suivent la documentation publique des
//! CLI. Ils sont testés sur un faux agent, pas contre les binaires réels
//! (Claude Code, Codex, Gemini CLI, OpenCode ne sont pas installés sur la
//! machine de développement). La détection dans le `PATH` et la supervision,
//! elles, sont testées.

use ih_core::config::{AgentConfig, AgentKind};
use ih_core::platform::find_in_path;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Description d'un adaptateur, exposée à l'interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub label: String,
    pub kind: AgentKind,
    pub program: String,
    pub enabled: bool,
    pub detected: bool,
    pub resolved_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAdapter {
    pub id: String,
    pub kind: AgentKind,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub enabled: bool,
    pub timeout_secs: u64,
}

impl AgentAdapter {
    pub fn from_config(config: &AgentConfig) -> Self {
        Self {
            id: config.id.clone(),
            kind: config.kind.clone(),
            label: config.label.clone(),
            program: config.program.clone(),
            args: config.args.clone(),
            enabled: config.enabled,
            timeout_secs: config.timeout_secs,
        }
    }

    pub fn detection(&self) -> Option<PathBuf> {
        find_in_path(&self.program)
    }

    pub fn info(&self) -> AgentInfo {
        let resolved = self.detection();
        AgentInfo {
            id: self.id.clone(),
            label: self.label.clone(),
            kind: self.kind.clone(),
            program: self.program.clone(),
            enabled: self.enabled,
            detected: resolved.is_some(),
            resolved_path: resolved.map(|p| p.display().to_string()),
        }
    }

    /// Construit le couple (programme, arguments) pour une instruction.
    ///
    /// Si `args` contient `{prompt}`, la substitution a lieu sur place ; sinon
    /// l'instruction est ajoutée en fin d'arguments selon le mode non
    /// interactif du CLI.
    pub fn build_command(&self, prompt: &str) -> (String, Vec<String>) {
        if self.args.iter().any(|a| a.contains("{prompt}")) {
            let args = self
                .args
                .iter()
                .map(|a| a.replace("{prompt}", prompt))
                .collect();
            return (self.program.clone(), args);
        }

        let mut args: Vec<String> = self.args.clone();
        match self.kind {
            // `claude -p "<prompt>"` : mode impression, une seule réponse.
            AgentKind::ClaudeCode => {
                args.push("-p".into());
                args.push(prompt.to_string());
            }
            // `codex exec "<prompt>"` : exécution non interactive.
            AgentKind::Codex => {
                if !args.iter().any(|a| a == "exec") {
                    args.push("exec".into());
                }
                args.push(prompt.to_string());
            }
            // `gemini -p "<prompt>"`.
            AgentKind::GeminiCli => {
                args.push("-p".into());
                args.push(prompt.to_string());
            }
            // `opencode run "<prompt>"`.
            AgentKind::Opencode => {
                if !args.iter().any(|a| a == "run") {
                    args.push("run".into());
                }
                args.push(prompt.to_string());
            }
            AgentKind::Custom => args.push(prompt.to_string()),
        }
        (self.program.clone(), args)
    }
}

/// Registre des adaptateurs configurés.
#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    adapters: Vec<AgentAdapter>,
}

impl AgentRegistry {
    pub fn from_configs(configs: &[AgentConfig]) -> Self {
        Self {
            adapters: configs.iter().map(AgentAdapter::from_config).collect(),
        }
    }

    pub fn list(&self) -> Vec<AgentInfo> {
        self.adapters.iter().map(|a| a.info()).collect()
    }

    pub fn get(&self, id: &str) -> Option<&AgentAdapter> {
        self.adapters.iter().find(|a| a.id == id)
    }

    pub fn enabled(&self) -> Vec<&AgentAdapter> {
        self.adapters.iter().filter(|a| a.enabled).collect()
    }

    pub fn detected(&self) -> Vec<&AgentAdapter> {
        self.enabled()
            .into_iter()
            .filter(|a| a.detection().is_some())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(kind: AgentKind, args: Vec<&str>) -> AgentAdapter {
        AgentAdapter {
            id: "test".into(),
            kind,
            label: "Test".into(),
            program: "sh".into(),
            args: args.into_iter().map(String::from).collect(),
            enabled: true,
            timeout_secs: 30,
        }
    }

    #[test]
    fn claude_utilise_le_mode_impression() {
        let (program, args) =
            adapter(AgentKind::ClaudeCode, vec![]).build_command("corrige le bug");
        assert_eq!(program, "sh");
        assert_eq!(args, vec!["-p", "corrige le bug"]);
    }

    #[test]
    fn codex_utilise_exec_une_seule_fois() {
        let (_, args) = adapter(AgentKind::Codex, vec![]).build_command("x");
        assert_eq!(args, vec!["exec", "x"]);
        let (_, args) = adapter(AgentKind::Codex, vec!["exec"]).build_command("x");
        assert_eq!(args, vec!["exec", "x"]);
    }

    #[test]
    fn gemini_et_opencode() {
        let (_, args) = adapter(AgentKind::GeminiCli, vec![]).build_command("p");
        assert_eq!(args, vec!["-p", "p"]);
        let (_, args) = adapter(AgentKind::Opencode, vec![]).build_command("p");
        assert_eq!(args, vec!["run", "p"]);
    }

    #[test]
    fn placeholder_prompt_respecte() {
        let (_, args) = adapter(AgentKind::Custom, vec!["--task", "{prompt}", "--yes"])
            .build_command("fais ceci");
        assert_eq!(args, vec!["--task", "fais ceci", "--yes"]);
    }

    #[test]
    fn detection_trouve_sh_mais_pas_un_binaire_absent() {
        let mut a = adapter(AgentKind::Custom, vec![]);
        assert!(a.detection().is_some());
        a.program = "binaire-inexistant-ih-xyz".into();
        assert!(a.detection().is_none());
        assert!(!a.info().detected);
    }

    #[test]
    fn registre_filtre_les_actifs() {
        let configs = vec![
            AgentConfig {
                id: "a".into(),
                kind: AgentKind::ClaudeCode,
                label: "A".into(),
                program: "sh".into(),
                enabled: true,
                ..Default::default()
            },
            AgentConfig {
                id: "b".into(),
                kind: AgentKind::Codex,
                label: "B".into(),
                program: "sh".into(),
                enabled: false,
                ..Default::default()
            },
        ];
        let registry = AgentRegistry::from_configs(&configs);
        assert_eq!(registry.list().len(), 2);
        assert_eq!(registry.enabled().len(), 1);
        assert_eq!(registry.detected().len(), 1);
    }
}
