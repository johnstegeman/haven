# Skill Backends

haven delegates AI skill management to a pluggable backend. The backend controls how skills are fetched, cached, and deployed to platform directories. Everything else — skill declarations (`ai/skills/`), platform targeting (`ai/platforms.toml`), ownership tracking (`state.json`), and CLAUDE.md generation — is always managed by haven regardless of which backend is active.

## Listing available backends

```sh
haven ai backends
```

Output example:

```
Skill backends:
  ✓ native   (active) — built-in, zero dependencies
  ✗ skillkit — runner 'npx' not found — install Node.js or set runner = "bunx"
    akm      — not yet implemented
```

## Configuring a backend

Create `ai/config.toml` in your repository root (optional — the native backend is the default):

```toml
[skills]
backend = "native"   # "native" | "skillkit"
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

### skillkit

Delegates to the [SkillKit](https://skillkit.dev) CLI. Provides access to the SkillKit marketplace, cross-agent translation, and AI-powered skill recommendations.

**Prerequisites:** Node.js (for `npx`) or Bun (for `bunx`), with SkillKit installed globally:

```sh
npm install -g skillkit
# or
bun add -g skillkit
```

**Configuration:**

```toml
[skills]
backend      = "skillkit"
runner       = "npx"        # "npx" (default) | "bunx" | "bun" | /path/to/binary
timeout_secs = 60           # subprocess timeout (default: 60)
```

**How it works:**

On `haven apply --ai`, haven:
1. Builds a `.skills` manifest from your `ai/skills/` declarations:
   ```json
   [
     {"name": "pdf-processing", "source": "anthropics/skills/pdf-processing", "version": "latest"},
     {"name": "find-skills",    "source": "vercel-labs/skills/find-skills",    "version": "latest"}
   ]
   ```
2. Calls `skillkit team install --manifest <tmpfile> --json` once (bulk, not per-skill).
3. Reads the JSON stdout to record deployed paths in `state.json`.
4. Regenerates CLAUDE.md from the deployed state.

**Manifest format:** `source` is the SkillKit marketplace ID — the `gh:` prefix is stripped and the path is preserved. `version` defaults to `"latest"` unless a pinned version is declared in `skill.toml`.

**Lock file behavior:** `haven.lock` does NOT record SHAs for SkillKit-managed skills. Version pinning is delegated to SkillKit internally. The `fetch()` step returns a synthetic `sha: "managed-by-skillkit"` and is otherwise a no-op — SkillKit handles downloading during deployment.

**Source restrictions:** SkillKit only supports `gh:` sources. Skills declared with `repo:` or `dir:` sources cause an immediate error when the SkillKit backend is active.

**Unavailability behavior:** If the configured runner is not on PATH, `haven apply` exits immediately:

```
error: skill backend 'skillkit' requires 'npx' but it was not found on PATH
hint: install Node.js, or set `runner = "bunx"` in ai/config.toml
```

Haven never silently falls back to the native backend — the configuration is always honored exactly.

**Runner detection:** If `runner` is `"npx"`, `"bunx"`, or `"bun"`, haven prepends `"skillkit"` to the argument list (`npx skillkit team install ...`). If `runner` is an absolute or relative path (contains `/`), haven calls it directly — useful for a standalone `skillkit` binary.

**When to use:** when you want access to the SkillKit marketplace, cross-agent skill translation, or `skillkit recommend` for discovery. Note: `haven.lock` no longer pins skill versions for SkillKit-managed skills when you switch to this backend.

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
| Collision detection warnings | haven (native) / not available (skillkit) |
| `haven ai discover` / `search` / `scan` | haven |
| SHA lock file | haven (native only) |
| Fetch + cache | haven (native) / skillkit (skillkit) |

## Switching backends

Your skill declarations (`ai/skills/`), platform config (`ai/platforms.toml`), and deployed skill names stay exactly the same when switching backends. Only `ai/config.toml` changes.

### native → skillkit

1. Install SkillKit:
   ```sh
   npm install -g skillkit
   # or: bun add -g skillkit
   ```

2. Create or update `ai/config.toml`:
   ```toml
   [skills]
   backend = "skillkit"
   runner  = "npx"   # or "bunx" if you installed via bun
   ```

3. Run apply:
   ```sh
   haven apply --ai
   ```
   haven generates a `.skills` manifest from your existing `ai/skills/` declarations and calls `skillkit team install`. Your skills are redeployed to the same `skills_dir` locations as before.

4. (Optional) Commit the change:
   ```sh
   git add ai/config.toml && git commit -m "chore: switch to skillkit backend"
   ```
   Note: `haven.lock` SHA entries for your skills are no longer updated by haven when using SkillKit. They become stale but are harmlessly ignored.

### skillkit → native

1. Update `ai/config.toml`:
   ```toml
   [skills]
   backend = "native"
   ```
   Or remove `ai/config.toml` entirely — `native` is the default.

2. Run apply:
   ```sh
   haven apply --ai
   ```
   haven fetches each skill directly from its `gh:` source, verifies SHA, and redeploys. SHA entries in `haven.lock` are rebuilt from scratch.

### Checking what's active

```sh
haven ai backends
```

The active backend is marked with `(active)`. Use this to confirm the switch took effect.
