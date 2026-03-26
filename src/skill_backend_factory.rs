/// Factory for instantiating the configured `SkillBackend`.
///
/// Reads `AiConfig`, checks availability of the chosen backend, and returns
/// a `Box<dyn SkillBackend>`. Fails loudly if the configured backend is
/// unavailable — no silent fallback.
///
/// Backend availability:
///   NativeBackend       — always available (built-in, zero deps)
///   AgentSkillsBackend  — available when `skills` binary is on PATH
///   AkmBackend          — not yet implemented; always errors
use anyhow::Result;
use std::path::Path;
use std::time::Duration;

use crate::ai_config::{AiConfig, BackendKind};
use crate::skill_backend::SkillBackend;
use crate::skill_backend_agentskills::AgentSkillsBackend;
use crate::skill_backend_native::NativeBackend;
use crate::util::is_on_path;

/// Instantiate the backend specified in `config`.
///
/// `state_dir` is passed to `NativeBackend` as its cache root.
/// Returns an error if the backend is unavailable or not yet implemented.
pub fn create_backend(config: &AiConfig, state_dir: &Path) -> Result<Box<dyn SkillBackend>> {
    match config.backend {
        BackendKind::Native => Ok(Box::new(NativeBackend::new(state_dir))),

        BackendKind::AgentSkills => {
            let runner = config.runner.clone();
            if !is_on_path(&runner) {
                anyhow::bail!(
                    "skill backend 'agent-skills' requires '{}' on PATH\n\
                     hint: install agent-skills-cli with: npm install -g agent-skills-cli",
                    runner
                );
            }
            Ok(Box::new(AgentSkillsBackend::new(
                runner,
                Duration::from_secs(config.timeout_secs),
            )))
        }

        BackendKind::Akm => {
            anyhow::bail!(
                "skill backend 'akm' is not yet implemented in this version of haven\n\
                 hint: switch to the native backend: echo 'backend = \"native\"' >> ai/config.toml"
            )
        }
    }
}

/// Return a human-readable list of known backends and their availability status,
/// for use by `haven ai backends`.
pub struct BackendInfo {
    pub name: &'static str,
    pub available: bool,
    pub note: String,
}

pub fn list_backends() -> Vec<BackendInfo> {
    let agent_skills_available = is_on_path("skills");
    let agent_skills_note = if agent_skills_available {
        "runner 'skills' found on PATH".to_string()
    } else {
        "requires agent-skills-cli (npm install -g agent-skills-cli)".to_string()
    };

    vec![
        BackendInfo {
            name: "agent-skills",
            available: agent_skills_available,
            note: agent_skills_note,
        },
        BackendInfo {
            name: "native",
            available: true,
            note: "built-in, zero dependencies".to_string(),
        },
        BackendInfo {
            name: "akm",
            available: false,
            note: "not yet implemented".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_config::BackendKind;
    use tempfile::TempDir;

    fn native_config() -> AiConfig {
        AiConfig {
            backend: BackendKind::Native,
            ..AiConfig::default()
        }
    }

    #[test]
    fn factory_returns_native_by_default() {
        let dir = TempDir::new().unwrap();
        let cfg = AiConfig::default();
        let backend = create_backend(&cfg, dir.path()).unwrap();
        assert_eq!(backend.name(), "native");
        assert!(backend.is_available());
    }

    #[test]
    fn factory_returns_native_when_configured() {
        let dir = TempDir::new().unwrap();
        let backend = create_backend(&native_config(), dir.path()).unwrap();
        assert_eq!(backend.name(), "native");
    }

    #[test]
    fn factory_errors_for_akm_backend() {
        let dir = TempDir::new().unwrap();
        let cfg = AiConfig {
            backend: BackendKind::Akm,
            ..AiConfig::default()
        };
        let err = create_backend(&cfg, dir.path()).err().expect("should have failed");
        let msg = format!("{err:#}");
        assert!(msg.contains("not yet implemented"), "should say not yet implemented: {msg}");
    }
}
