# Swappable Skill Backends for Haven

> **Status (2026-03-26):** The SkillKitBackend implemented in Phase 3 has been removed.
> SkillKit is project-focused (it installs skills into a specific project) and is not
> compatible with Haven's dotfiles-style global skill management. The NativeBackend
> remains the only supported backend. The AkmBackend slot is still reserved for a
> future implementation.



## Problem

Haven's AI skills layer currently has a single, built-in implementation: it fetches skills from GitHub (with SHA-256 locking), deploys them via symlink or copy, tracks ownership in `state.json`, and generates `CLAUDE.md`. This works well, but it means Haven must evolve its own skills ecosystem rather than leveraging the rapidly growing set of external skill managers.

The AI skill management space has exploded in 2025–2026:
- **SkillKit** — cross-agent translation, 400K+ skill registry, AI-powered recommendations
- **akm (agentikit)** — pluggable providers, 6 asset types (skills, scripts, agents, knowledge, memories)
- **skm (skills-management)** — sparse checkout, simple CLI
- **agentskills** — official Anthropic open specification (14.1k stars), the emerging standard
- Multiple desktop GUI managers (xingkongliang/skills-manager with 372 stars, etc.)

None of these tools are as mature as Homebrew. But the space is moving fast, and building Haven's entire skill pipeline in-house forfeits the marketplace, cross-agent translation, and community ecosystem these tools provide.

**The core insight:** Haven already follows the pattern of "declare intent in config → invoke backend → backend handles mechanics" for packages (Homebrew) and runtime versions (mise). Skills should follow the same pattern.

## Proposed Architecture

### Design principle

Haven declares *what* skills to use. A configurable backend handles *how* to fetch, cache, and deploy them. The interface between Haven and any backend is defined by the **agentskills specification** — the official Anthropic open standard that these tools are converging on.

```
haven apply
  └── SkillManager
        ├── reads: ai/skills/<name>/skill.toml  (what to install)
        ├── reads: ai/platforms.toml            (where to deploy)
        └── delegates to: SkillBackend trait
              ├── NativeBackend    [default, built-in, zero deps]
              ├── SkillKitBackend  [opt-in: requires skillkit on PATH]
              └── AkmBackend       [opt-in: requires akm on PATH]
```

### Configuration

In `ai/config.toml` (new file, optional — defaults to `native`):

```toml
[skills]
backend = "native"      # "native" | "skillkit" | "akm"
```

Or per-skill override in `ai/skills/<name>/skill.toml`:

```toml
source   = "gh:anthropics/skills/pdf-processing"
platforms = ["claude-code"]
backend  = "skillkit"   # override for this skill only
```

---

## The agentskills Spec as Interface Contract

The [agentskills specification](https://agentskills.io/specification) (Anthropic, Apache 2.0) defines the minimal standard all compliant skills share:

### Skill format

```
skill-name/
├── SKILL.md          # Required: YAML frontmatter + markdown instructions
├── scripts/          # Optional: executable code
├── references/       # Optional: detailed docs
└── assets/           # Optional: templates, data
```

**SKILL.md frontmatter (required fields):**
```yaml
---
name: pdf-processing          # 1–64 chars, lowercase-hyphenated, matches dir name
description: "..."            # 1–1024 chars, when to use this skill
license: Apache-2.0           # optional
compatibility: "Python 3.9+"  # optional
metadata:
  author: example-org
  version: "1.0"
---
```

### What the spec does NOT define (platform concerns)

- How skills are fetched or cached
- CLI verbs (install, sync, list, remove)
- Lockfile format
- Multi-agent deployment
- Versioning beyond optional `metadata.version`

These are platform concerns. Haven's `SkillBackend` trait covers them.

---

## The `SkillBackend` Trait

```rust
pub trait SkillBackend: Send + Sync {
    /// Download a skill from source into local cache.
    /// Returns the content SHA for lock file recording.
    fn fetch(
        &self,
        source: &SkillSource,
        expected_sha: Option<&str>,
    ) -> Result<FetchResult>;

    /// Deploy a cached skill to a platform's skills directory.
    fn deploy(
        &self,
        skill: &ResolvedSkill,
        target: &DeploymentTarget,
    ) -> Result<DeployResult>;

    /// Deploy all skills in a single operation.
    ///
    /// Default implementation loops over `deploy()` — NativeBackend uses this.
    /// SkillKit overrides this to generate a `.skills` manifest and call
    /// `skillkit team install` once, because SkillKit's API is bulk-only:
    /// calling it N times per-skill would be incorrect and slow.
    fn deploy_all(
        &self,
        skills: &[(&ResolvedSkill, &DeploymentTarget)],
    ) -> Result<Vec<DeployResult>> {
        skills.iter().map(|(s, t)| self.deploy(s, t)).collect()
    }

    /// Remove a deployed skill from a platform directory.
    fn undeploy(&self, target: &Path) -> Result<()>;

    /// Parse and validate a SKILL.md, returning metadata.
    fn validate(&self, skill_path: &Path) -> Result<SkillMetadata>;

    /// List skills currently in cache.
    fn list_cached(&self) -> Result<Vec<CachedSkillInfo>>;

    /// Remove a skill from cache.
    fn evict(&self, source_key: &str) -> Result<()>;

    /// Human-readable name for error messages.
    fn name(&self) -> &str;

    /// Whether this backend is available in the current environment.
    fn is_available(&self) -> bool;
}
```

**Supporting types:**

```rust
pub struct FetchResult {
    pub cached_path: PathBuf,   // Where the skill now lives in cache
    pub sha: String,             // Content SHA (git SHA or tarball SHA-256)
    pub was_cached: bool,        // True if cache hit, false if downloaded
}

pub struct ResolvedSkill {
    pub name: String,
    pub cached_path: PathBuf,
    pub sha: String,
    pub metadata: SkillMetadata,
}

pub struct DeploymentTarget {
    pub platform_id: String,
    pub skills_dir: PathBuf,
    pub deploy_method: DeployMethod,  // Symlink | Copy
}

pub struct DeployResult {
    pub target_path: PathBuf,
    pub was_collision: bool,         // True if path existed but wasn't Haven-owned
    pub deployed: bool,
}

pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: HashMap<String, String>,
}
```

---

## Backend Implementations

### 1. NativeBackend (default)

Haven's current implementation, extracted into the trait. Zero additional dependencies.

- Fetch: git sparse checkout → tarball fallback, SHA-256 verification
- Deploy: symlink or copy, collision detection, state tracking
- Cache: `~/.haven/skills/{owner}--{repo}[--{subpath}]/`
- Lock: `haven.lock` TOML, SHA pinned per source

**Advantages:** No external runtime, reproducible, cryptographically verified, Haven owns the full pipeline.

### 2. SkillKitBackend

Shells out to the `skillkit` CLI (requires Node.js + `npm install -g skillkit`).

```
haven ai add gh:anthropics/skills/pdf   →  skillkit install anthropics/skills/pdf
haven apply --ai                         →  skillkit team install --manifest .skills
haven ai update pdf                      →  skillkit update pdf
```

Haven generates the `.skills` manifest from `ai/skills/<name>/skill.toml` declarations, calls `skillkit team install`, then reads the deployed results to update `state.json` and regenerate `CLAUDE.md`.

**Advantages:** 400K+ marketplace, cross-agent translation, AI-powered recommendations (`skillkit recommend`).

**Runner configuration:** Invocation is runner-agnostic. `ai/config.toml`:
```toml
[skills]
backend = "skillkit"
runner = "npx"   # "npx" | "bunx" | "bun" | path to binary (default: "npx")
```
If the configured runner is not on PATH, `haven apply` exits immediately with an actionable error:
```
error: skill backend 'skillkit' requires 'npx' but it was not found on PATH
hint: install Node.js, or set `runner = "bunx"` in ai/config.toml
```
No silent fallback to NativeBackend — the user explicitly chose this backend.

**Tradeoffs:** External runtime required; Haven partially loses per-skill deployment control; SkillKit's version pinning (`"version": "1.2.0"`) replaces Haven's SHA-256 lock for SkillKit-managed skills — `haven.lock` does NOT record SHAs for these.

**`fetch()` for SkillKitBackend:** Returns a synthetic `FetchResult { sha: "managed-by-skillkit", was_cached: true }`. Actual downloading is handled by SkillKit internally during `deploy_all()`. Haven does not manage the cache for SkillKit skills.

**`.skills` manifest format:**
```json
[
  {"name": "pdf-processing", "source": "anthropics/skills/pdf-processing", "version": "latest"},
  {"name": "find-skills",    "source": "vercel-labs/skills/find-skills",    "version": "latest"}
]
```
`source` is the SkillKit marketplace ID — the `gh:` prefix is stripped and the path is preserved. `version` defaults to `"latest"` unless the skill.toml specifies a pinned version.

**Non-`gh:` source handling:** SkillKitBackend only supports `gh:` sources. Skills declared with `repo:` or `dir:` sources cause an immediate error if SkillKit is the active backend:
```
error: skill 'myskill' uses source 'repo:' which is not supported by the SkillKit backend
hint: switch to the native backend, or re-source the skill from a GitHub registry
```

**`deploy()` semantics for SkillKitBackend:** SkillKit is bulk-only — `deploy()` must not be called directly. The implementation returns an error:
```rust
fn deploy(&self, ..) -> Result<DeployResult> {
    Err(anyhow!("SkillKitBackend: use deploy_all() — SkillKit requires bulk deployment"))
}
```
`deploy_all()` is the correct entry point for all SkillKit deployments. The `apply_ai_skills()` caller always uses `deploy_all()`.

**`fetch()` semantics for SkillKitBackend:** Returns immediately with a synthetic result. The `expected_sha` parameter is ignored (SkillKit manages its own cache):
```rust
fn fetch(&self, _source: &SkillSource, _expected_sha: Option<&str>) -> Result<FetchResult> {
    Ok(FetchResult { sha: "managed-by-skillkit".into(), was_cached: true, cached_path: PathBuf::new() })
}
```

**Subprocess safety:**
- Timeout: 60 seconds per `skillkit` invocation (configurable via `[skills] timeout_secs`)
- On timeout: `Child::kill()` is called before returning the timeout error — no orphaned subprocess
- The `.skills` manifest is written to a temp file and passed to SkillKit; never committed to the repo
- Manifest write is atomic (write to `.skills.tmp`, rename on success)
- `deploy_all()` returns `DeployResult[]` by parsing SkillKit's stdout (JSON list of deployed paths, via `skillkit team install --json`). Filesystem diff is NOT used — it's fragile under partial failures. If `--json` flag is unavailable, fall back to listing `skills_dir` entries that weren't present in state.json before the call.
- `state.json` is NOT updated if skillkit exits non-zero
- `skillkit team install` is idempotent — re-running with the same manifest re-deploys to the same locations; already-present skills are a no-op. This means partial-failure recovery is safe: the next `haven apply` re-runs the full manifest.
- Race protection: Haven's existing `apply.lock` prevents concurrent `haven apply` runs regardless of backend
- `was_collision` in `DeployResult` is always `false` for SkillKit (SkillKit stdout does not expose collision info). See TODOS.md for the planned pre-scan mitigation.

**Degraded mode:** If the user has configured `backend = "skillkit"` but SkillKit becomes unavailable (e.g., after a Node.js upgrade or machine reinstall), `haven apply` exits with:
```
error: skill backend 'skillkit' is configured but unavailable
hint: reinstall with `npm install -g skillkit`, or switch to the native backend:
      echo 'backend = "native"' >> ai/config.toml
```

### 3. AkmBackend (future)

Shells out to `akm` (requires Bun or standalone binary).

Useful for users who want akm's broader asset model: skills + scripts + agents + knowledge bases + memories as a unified package.

Not prioritized for initial implementation — include the trait slot now, implement when akm matures.

---

## Implementation Plan

### Phase 1: Extract NativeBackend (no behavior change)

**Goal:** Refactor the current skill pipeline into the `SkillBackend` trait without changing any behavior. This is pure internal restructuring.

**Files changed:**
- `src/skill_backend.rs` — new: trait definition + supporting types
- `src/skill_backend_native.rs` — new: NativeBackend wrapping `SkillCache` (private dep; `skill_cache.rs` stays separate)
- `src/ai_skill.rs` — simplified: deploy_skill() moved to NativeBackend; SkillsConfig stays
- `src/skill_cache.rs` — unchanged; remains NativeBackend's private dependency
- `src/commands/apply.rs` — updated: `apply_ai_skills()` wired to call `NativeBackend` via trait

**Acceptance criteria:**
- All existing tests pass unchanged
- `haven apply` behavior is byte-for-byte identical
- `haven.lock` format unchanged
- No new config keys required

### Phase 2: Backend selection infrastructure

**Goal:** Add backend selection to config and wire the factory.

**Files changed:**
- `src/ai_config.rs` — new: `AiConfig` struct with `backend`, `runner`, `timeout_secs` fields, reads from `ai/config.toml`
- `src/skill_backend_factory.rs` — new: instantiates the right backend, checks availability, clears stale lock entries on backend switch
- `src/commands/ai.rs` — pass selected backend through to all skill operations
- `src/commands/apply.rs` — updated: reads `AiConfig`, calls factory, passes `Box<dyn SkillBackend>` to `apply_ai_skills()`
- `src/util.rs` — new (or expanded): `pub fn is_on_path(name: &str) -> bool` (extracted from `commands/ai.rs::which_on_path`)

**New config file** (`ai/config.toml`, optional):
```toml
[skills]
backend = "native"   # default; "skillkit" | "akm" also valid
```

**Acceptance criteria:**
- `haven apply` with no `ai/config.toml` behaves identically to Phase 1
- `haven apply` with `backend = "native"` explicitly behaves identically
- Unknown backend name gives a clear error with available options listed
- `haven ai backends` (new subcommand) lists available backends + whether each is detected

### Phase 3: SkillKitBackend

**Goal:** Implement `SkillKitBackend` as the first non-native backend.

**Files changed:**
- `src/skill_backend_skillkit.rs` — new: SkillKitBackend implementation

**Behavior:**
1. `is_available()`: checks `which skillkit` succeeds
2. On `haven ai add <source>`: translates source to skillkit format, runs `skillkit install <source>`, records in `ai/skills/<name>/skill.toml`
3. On `haven apply --ai`: generates `.skills` manifest from `ai/skills/` declarations, runs `skillkit team install`, reads deployed paths, updates `state.json`, regenerates `CLAUDE.md`
4. On `haven ai update [name]`: runs `skillkit update [name]`
5. On `haven ai remove <name>`: runs `skillkit remove <name>`, cleans up `ai/skills/<name>/`

**Degraded mode:** If `skillkit` disappears from PATH after configuration, warn clearly and suggest switching back to `backend = "native"` or running `npm install -g skillkit`.

**Acceptance criteria:**
- `haven apply --ai` with SkillKit backend installs skills to the same `skills_dir` as native backend
- `CLAUDE.md` is generated correctly regardless of backend
- `haven status` correctly reports deployed skills (by scanning `skills_dir` on filesystem)
- Error when `skillkit` not available is actionable — includes runner hint and downgrade instruction
- `haven apply --ai --dry-run --show-manifest` prints the `.skills` JSON without invoking skillkit
- `haven ai backends` lists skillkit as available/unavailable with version and runner
- `haven ai search <query>` delegates to `skillkit marketplace` when backend = skillkit (via `search()` optional capability — default `Err` falls back to skills.sh)

**Integration test:** End-to-end with a real `skillkit` installation:
```
given: backend = "skillkit", runner = "npx", one skill declared in ai/skills/
when: haven apply --ai
then: skill directory exists in ~/.claude/skills/, CLAUDE.md updated, state.json records ownership
```

### Phase 4: Documentation + migration guide

- Update `docs/guides/ai-skills.md` with backend selection docs
- Add `docs/reference/skill-backends.md`
- Note: existing repos need no changes (native backend is default)

---

## What Haven retains regardless of backend

- `ai/skills/<name>/skill.toml` as the declaration format (source of truth in the repo)
- `ai/platforms.toml` for platform targeting
- SHA lock file integration for the NativeBackend
- `state.json` ownership tracking (populated by whichever backend deploys)
- `CLAUDE.md` generation (Haven always drives this from deployed state)
- Collision detection warnings
- `haven ai discover` / `haven ai search` / `haven ai scan` (backend-independent)

---

## What Haven does NOT need to build

With SkillKit as an optional backend, Haven does not need to build:
- Its own skill marketplace or registry UI
- Cross-agent skill translation
- AI-powered skill recommendations
- Team sync features

These become available by choosing `backend = "skillkit"`.

---

## Resolved design decisions

1. **Per-skill backend override:** Deferred. Global backend sufficient for v1. Add when there's user demand.

2. **SkillKit manifest placement:** Temp-only. The `.skills` manifest is generated to a temp file at apply time and never committed to the repo. It is an implementation detail of the SkillKitBackend, not a user-facing config.

3. **Lock file for SkillKit-managed skills:** Haven's `haven.lock` does NOT record SHAs for SkillKit-managed skills. Locking is delegated to SkillKit (version pinning in `.skills`). The `fetch()` method for SkillKitBackend returns a synthetic `FetchResult { sha: "managed-by-skillkit" }`.

4. **AkmBackend:** Register the slot now (stub with `is_available() → false` and a clear "not yet implemented" error). Exact error message:
   ```
   error: skill backend 'akm' is not yet implemented in this version of haven
   hint: switch to the native backend: echo 'backend = "native"' >> ai/config.toml
   ```
   Implement when akm matures.

7. **Backend-switch lock clearing:** When the factory detects a backend switch (active backend differs from last backend recorded in lock metadata), it clears all skill SHA entries from `haven.lock` for affected skills before deploying. This prevents `SkillCache::ensure()` from triggering a spurious SHA mismatch error when switching from SkillKit (sha = "managed-by-skillkit") back to native. The `haven.lock` will record a `last_backend` key for this purpose.

8. **`haven ai update` for SkillKit:** Runs `skillkit update [name]`. Since `haven.lock` doesn't record SHAs for SkillKit skills, the lock is not consulted or updated. SkillKit manages versioning internally.

9. **Unit test strategy for SkillKitBackend subprocess:** Use a fake binary via the configured `runner`. In tests, set `runner = "sh"` and point to `tests/fixtures/fake-skillkit.sh` which outputs expected JSON and exits 0. Multiple variants for error paths (exit 1, timeout, `--json` unavailable).

5. **`haven status` with external backends:** Filesystem-based scan of `skills_dir` — backend-agnostic. State.json records what Haven knows was deployed; for SkillKit, this is populated from SkillKit's stdout (`--json` flag) not filesystem diff.

6. **`search()` on the trait:** Optional capability, not a required trait method. Implemented as a separate `fn search(&self, query: &str) -> Result<Vec<SkillSearchResult>>` with a default `Err("search not supported by this backend")`. The `haven ai search` command calls this if the backend supports it, otherwise falls back to skills.sh directly.

---

## Ecosystem context

| Tool | Stars | Runtime | Role |
|---|---|---|---|
| agentskills spec | 14.1k | — | Interface standard |
| SkillKit | 649 | Node.js | Feature-rich CLI, marketplace |
| xingkongliang/skills-manager | 372 | Tauri/Rust | Desktop GUI, 15+ agents |
| skm | low | Node.js | Minimal CLI |
| akm | low | Bun | Pluggable, broad asset model |

The space is nascent. No tool has Homebrew-level maturity. The swappable backend design is the correct response: Haven stays useful regardless of which tool wins.

---

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 1 | issues_open | 6 proposals, 5 accepted, 1 deferred |
| Codex Review | `/codex review` | Independent 2nd opinion | 1 | issues_found | 8 findings (outside voice, Claude subagent) |
| Eng Review | `/plan-eng-review` | Architecture & tests (required) | 1 | clean (PLAN) | 12 issues, 0 unresolved, 0 critical gaps |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | — | — |

**UNRESOLVED:** 0 decisions outstanding
**VERDICT:** CEO + ENG CLEARED — ready to implement. Start with Phase 1 (NativeBackend extraction).
