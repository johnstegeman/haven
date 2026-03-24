/// Factory for instantiating the configured `SkillBackend`.
///
/// Reads `AiConfig`, checks availability of the chosen backend, and returns
/// a `Box<dyn SkillBackend>`. Fails loudly if the configured backend is
/// unavailable — no silent fallback.
///
/// Backend availability:
///   NativeBackend  — always available (built-in, zero deps)
///   SkillKitBackend — requires runner (e.g. "npx") on PATH
///   AkmBackend      — not yet implemented; always errors
use anyhow::Result;
use std::path::Path;

use crate::ai_config::{AiConfig, BackendKind};
use crate::skill_backend::SkillBackend;
use crate::skill_backend_native::NativeBackend;
use crate::util::is_on_path;

/// Instantiate the backend specified in `config`.
///
/// `state_dir` is passed to `NativeBackend` as its cache root.
/// Returns an error if the backend is unavailable or not yet implemented.
pub fn create_backend(config: &AiConfig, state_dir: &Path) -> Result<Box<dyn SkillBackend>> {
    match config.backend {
        BackendKind::Native => Ok(Box::new(NativeBackend::new(state_dir))),

        BackendKind::SkillKit => {
            if !is_on_path(&config.runner) {
                anyhow::bail!(
                    "skill backend 'skillkit' requires '{}' but it was not found on PATH\n\
                     hint: install Node.js, or set `runner = \"bunx\"` in ai/config.toml",
                    config.runner
                );
            }
            // SkillKitBackend will be implemented in Phase 3.
            // For now, check availability and return a placeholder error so the
            // factory skeleton compiles and the availability path is tested.
            anyhow::bail!(
                "skill backend 'skillkit' is not yet implemented in this build of haven\n\
                 hint: use `backend = \"native\"` in ai/config.toml"
            )
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

pub fn list_backends(config: &AiConfig) -> Vec<BackendInfo> {
    let skillkit_runner = config.runner.as_str();
    let skillkit_available = is_on_path(skillkit_runner);

    vec![
        BackendInfo {
            name: "native",
            available: true,
            note: "built-in, zero dependencies".to_string(),
        },
        BackendInfo {
            name: "skillkit",
            available: skillkit_available,
            note: if skillkit_available {
                format!("runner '{}' found on PATH", skillkit_runner)
            } else {
                format!(
                    "runner '{}' not found — install Node.js or set runner = \"bunx\"",
                    skillkit_runner
                )
            },
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
            runner: "npx".to_string(),
            timeout_secs: 60,
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
            runner: "bun".to_string(),
            timeout_secs: 60,
        };
        let err = create_backend(&cfg, dir.path()).err().expect("should have failed");
        let msg = format!("{err:#}");
        assert!(msg.contains("not yet implemented"), "should say not yet implemented: {msg}");
    }

    #[test]
    fn factory_errors_loudly_for_unavailable_skillkit_runner() {
        let dir = TempDir::new().unwrap();
        let cfg = AiConfig {
            backend: BackendKind::SkillKit,
            runner: "no-such-binary-xyz".to_string(),
            timeout_secs: 60,
        };
        let err = create_backend(&cfg, dir.path()).err().expect("should have failed");
        let msg = format!("{err:#}");
        assert!(msg.contains("no-such-binary-xyz"), "should name the runner: {msg}");
        assert!(msg.contains("hint:"), "should include a hint: {msg}");
    }
}
