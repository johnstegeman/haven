# Skill Backends

haven delegates AI skill management to a pluggable backend. The backend controls how skills are fetched, cached, and deployed to platform directories. Everything else — skill declarations (`ai/skills/`), platform targeting (`ai/platforms.toml`), ownership tracking (`state.json`), and CLAUDE.md generation — is always managed by haven regardless of which backend is active.

## Listing available backends

```sh
haven ai backends
```

Output example (with agent-skills-cli installed):

```
Skill backends:

 * ✓ agent-skills — runner 'skills' found on PATH
   ✓ native       — built-in, zero dependencies
   ✗ akm          — not yet implemented
```

## Configuring a backend

Create `ai/config.toml` in your repository root (optional — the native backend is the default):

```toml
[skills]
backend = "native"   # "native" | "agent-skills" | "akm"
```

## Backend reference

### native

**Default.** Built-in, zero external dependencies.

- **Fetch:** git sparse checkout with tarball fallback; SHA-256 content verification on every fetch.
- **Cache:** `~/.haven/skills/{owner}--{repo}[--{subpath}]/` — one directory per skill.
- **Deploy:** symlink (default) or copy to the platform's `skills_dir`. Collision detection: skills owned by another tool are skipped with a warning.
- **Lock:** every `gh:` skill is pinned by SHA in `haven.lock`. Supply-chain-safe: a content hash mismatch is a hard error, not a warning.

**Configuration:** none required.

**When to use:** default for all users. Fully reproducible and offline-capable after first fetch.

---

### agent-skills

Delegates fetch and deployment to [agent-skills-cli](https://www.agentskills.in/). Gives access to 175K+ marketplace skills with cross-agent deployment. Requires Node.js 18+ and the `skills` binary on PATH.

```sh
npm install -g agent-skills-cli
```

```toml
[skills]
backend      = "agent-skills"
runner       = "skills"   # default; accepts full path if not on PATH
timeout_secs = 120        # default; increase on slow connections
```

- **Fetch:** no-op — agent-skills-cli downloads during deploy.
- **Cache:** `~/.skills/` (managed by agent-skills-cli; haven does not touch it).
- **Deploy:** runs `skills install <source> -g -a <agent> -y`, then verifies the target path exists.
- **Lock:** SHA is recorded as `"managed-by-agent-skills"` in `haven.lock`; version authority is `~/.skills/skills.lock`.
- **Undeploy:** removes the deployed symlink/directory directly; does not call `skills uninstall` (preserves the shared `~/.skills/` cache for other agents).

**Source format:**

| haven declaration | agent-skills-cli invocation |
|-------------------|-----------------------------|
| `gh:owner/repo/skill` | `skills install owner/repo -s skill -g -a <agent> -y` |
| `gh:owner/repo` | `skills install owner/repo -g -a <agent> -y` |
| `gh:owner/repo@ref` | same as above (`@ref` dropped — version managed by agent-skills-cli) |
| `dir:~/path` | `skills install <expanded-path> -g -a <agent> -y` |
| `repo:` | error (not supported by this backend) |

**Search:** `haven ai search` routes to the agent-skills marketplace when this backend is active.

**When to use:** when you want access to the agent-skills marketplace or need multi-agent deployment without managing a separate fetch cache.

---

### akm

Not yet implemented. Reserved for a future [akm](https://github.com/agentikit/akm) backend.

```
error: skill backend 'akm' is not yet implemented in this version of haven
hint: switch to the native backend: echo 'backend = "native"' >> ai/config.toml
```

## What haven always manages

Regardless of backend:

| Concern | Who handles it |
|---------|---------------|
| Skill declarations (`ai/skills/`) | haven |
| Platform targeting (`ai/platforms.toml`) | haven |
| Ownership tracking (`state.json`) | haven |
| CLAUDE.md generation | haven |
| Collision detection warnings | haven |
| `haven ai discover` / `search` / `scan` | haven |
| SHA lock file (`haven.lock`) | haven |
| Fetch + cache | native backend only; agent-skills backend delegates to `~/.skills/` |
