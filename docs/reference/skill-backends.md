# Skill Backends

haven delegates AI skill management to a pluggable backend. The backend controls how skills are fetched, cached, and deployed to platform directories. Everything else — skill declarations (`ai/skills/`), platform targeting (`ai/platforms.toml`), ownership tracking (`state.json`), and CLAUDE.md generation — is always managed by haven regardless of which backend is active.

## Listing available backends

```sh
haven ai backends
```

Output example:

```
Skill backends:

 * ✓ native — built-in, zero dependencies
   ✗ akm    — not yet implemented
```

## Configuring a backend

Create `ai/config.toml` in your repository root (optional — the native backend is the default):

```toml
[skills]
backend = "native"   # "native" | "akm"
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
| SHA lock file | haven |
| Fetch + cache | haven |
