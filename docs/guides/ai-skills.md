# AI Skills

haven manages AI agent skills — reusable instruction sets for Claude Code, Codex, and other platforms — alongside your dotfiles and packages. Skills are declared in `ai/skills/`, fetched from GitHub with SHA pinning, and deployed to platform-specific directories on `haven apply`.

## What are skills?

Skills are markdown instruction files that extend what your AI coding assistant can do. Claude Code skills, for example, are markdown files in `~/.claude/skills/` that appear in the assistant's context.

haven treats skills as first-class managed artifacts: you declare them, version-pin them, and deploy them like any other part of your environment.

## Discovering installed platforms

```sh
haven ai discover
```

Scans this machine for installed AI agent platforms (Claude Code, Codex, Cursor, etc.) and offers to update `ai/platforms.toml`.

```toml
# ai/platforms.toml
active = ["claude-code"]

[platform.claude-code]
skills_dir = "~/.claude/skills"
```

## Adding a skill

```sh
haven ai add gh:anthropics/skills/pdf-processing@v1.0
haven ai add gh:me/my-commands@main --platforms claude-code
haven ai add dir:~/projects/my-skill
```

This creates `ai/skills/<name>/skill.toml` and a blank `all.md` stub. It does **not** deploy immediately — run `haven apply --ai` afterward.

### Skill source formats

| Format | Example | Description |
|--------|---------|-------------|
| `gh:owner/repo` | `gh:anthropics/skills/pdf-processing` | GitHub repo or subdirectory. Optional `@ref` for branch/tag. |
| `dir:~/path` | `dir:~/projects/my-skill` | Local directory. Read directly, not cached. |

### Platform targeting

| Value | Meaning |
|-------|---------|
| `"all"` | All active platforms in `ai/platforms.toml` |
| `"cross-client"` | Only the cross-client platform (`~/.agents/skills/`) |
| `["claude-code"]` | Explicit list, filtered to active platforms |

### Deploy methods

| Method | Behavior |
|--------|----------|
| `symlink` (default) | Creates a symlink `{skills_dir}/{name}` → cache dir |
| `copy` | Copies the skill directory. For platforms that don't follow symlinks. |

## Skill directory structure

```
ai/skills/
  pdf-processing/
    skill.toml       ← source, platforms, deploy method
    all.md           ← snippet injected into every platform's config file
    claude-code.md   ← snippet injected only into Claude Code's CLAUDE.md
  my-commands/
    skill.toml
    all.md
```

**`skill.toml`** — declares source and targeting:

```toml
source    = "gh:anthropics/skills/pdf-processing@v1.0"
platforms = "all"
deploy    = "symlink"    # or "copy"
```

**`all.md`** — snippet content injected into every active platform's config file.

**`<platform>.md`** — platform-specific snippet (e.g. `claude-code.md` → `~/.claude/CLAUDE.md`).

## Deploying skills

```sh
haven apply --ai
```

Deploys all declared skills to their platform directories. For Claude Code, skill snippets from `all.md` and `claude-code.md` are injected into `~/.claude/CLAUDE.md` between HTML comment markers:

```html
<!-- haven managed start -->
...skill content...
<!-- haven managed end -->
```

If the config file has no markers and the session is interactive, you are prompted to add them. Subsequent applies keep the markers up to date.

## Fetching and updating skills

```sh
haven ai fetch              # download all skills to cache without deploying
haven ai fetch pdf-processing   # fetch one skill

haven ai update             # pull latest versions + update lock SHAs
haven ai update pdf-processing  # update one skill
```

**`fetch`** respects the current lock SHA — no-op if already cached at the right version.

**`update`** clears the lock SHA and pulls the latest, then records the new SHA. Use this to intentionally upgrade a skill.

## Removing a skill

```sh
haven ai remove pdf-processing
haven ai remove pdf-processing --yes    # skip confirmation
```

Removes `ai/skills/pdf-processing/` and optionally removes it from platform skill directories.

## Searching for skills

```sh
haven ai search pdf
haven ai search browser --limit 5
```

Searches the [skills.sh](https://skills.sh) registry. Results show the skill source in `gh:owner/repo/skill` format. Copy the source and pass it to `haven ai add`.

## Importing existing skills

If you already have skills installed in a skills directory and want to bring them under haven management:

```sh
haven ai scan ~/.claude/skills
haven ai scan ~/.agents/skills
```

For each unmanaged skill, haven tries to identify its GitHub source by inspecting git remotes or searching the skills.sh registry. You confirm, edit, or skip each one. Confirmed skills are added to `ai/skills/`.

```sh
haven ai scan ~/.claude/skills --dry-run    # preview only
```

## Backend selection

haven ships with a built-in skill backend (`native`) and supports external backends as opt-in alternatives. The backend controls how skills are fetched, cached, and deployed.

```sh
haven ai backends          # list all backends and their availability
```

Configure via `ai/config.toml` (optional — defaults to `native`):

```toml
[skills]
backend = "native"         # "native" (default) | "skillkit"
```

### native (default)

Built-in, zero dependencies. Haven fetches skills directly from GitHub with SHA-256 verification, deploys via symlink or copy, and records every installed skill in `haven.lock`.

No configuration required. This is what you get without any `ai/config.toml`.

### skillkit

Delegates to the [SkillKit](https://skillkit.dev) CLI for access to its 400K+ skill marketplace, cross-agent skill translation, and AI-powered recommendations.

**Prerequisites:** Node.js + `npm install -g skillkit` (or Bun).

```toml
[skills]
backend = "skillkit"
runner  = "npx"       # "npx" (default) | "bunx" | "bun" | path to binary
```

With SkillKit as backend, `haven apply --ai` generates a `.skills` manifest from your declared skills and calls `skillkit team install` once — a single bulk operation rather than per-skill deploys. State tracking and CLAUDE.md generation still happen in haven.

**Note:** `haven.lock` does not record SHAs for SkillKit-managed skills. Version pinning is delegated to SkillKit.

If SkillKit is configured but unavailable, `haven apply` exits immediately with an actionable error — haven never silently falls back to the native backend.

See [Skill Backends](../reference/skill-backends.md) for full configuration reference, per-backend behavior, and step-by-step switching instructions.

## Supply chain protection

Every `gh:` skill source is pinned by SHA256 in `haven.lock`. On cache miss, the fetched content is verified against the recorded SHA. A mismatch is a hard error:

```toml
# haven.lock (auto-generated — do not edit by hand)
[skill."gh:anthropics/skills/pdf-processing@v1.0"]
sha        = "abc123def456..."
fetched_at = "2026-03-21T10:00:00Z"
```

To accept an upgrade, run `haven ai update <name>`. This clears the old SHA, fetches the latest, and records the new SHA.

Commit `haven.lock` to your repo — it pins your exact skill versions for reproducible installs across machines.
