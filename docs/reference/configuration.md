# Configuration Reference

## `haven.toml`

The top-level configuration file at the repo root. All sections are optional.

```toml
# haven.toml

[profile.default]
modules = ["shell", "git", "packages"]

[profile.work]
extends = "default"
modules = ["work", "secrets"]

[data]
work_email    = "alice@corp.example"
kanata_path   = "/usr/local/bin/kanata"

[security]
allow = [
  "~/.config/gh/hosts.yml",
]

[vcs]
backend = "git"    # or "jj"

[telemetry]
enabled = false
```

### `[profile.<name>]`

Declares a profile — a named set of modules to apply.

| Field | Type | Description |
|-------|------|-------------|
| `modules` | string array | Module names to activate for this profile. |
| `extends` | string | Parent profile name. Parent's modules are applied first. |

### `[data]`

Custom variables available in all `.tmpl` files as `{{ data.<key> }}`.

Only flat string values are supported. Arrays and nested tables are not.

### `[security]`

| Field | Type | Description |
|-------|------|-------------|
| `allow` | string array | Glob patterns for files to skip in security scanning. Same syntax as `config/ignore`. |

### `[vcs]`

| Field | Type | Description |
|-------|------|-------------|
| `backend` | `"git"` or `"jj"` | VCS backend for clone and init operations. |

### `[telemetry]`

| Field | Type | Description |
|-------|------|-------------|
| `enabled` | bool | Enable local telemetry logging to `~/.haven/telemetry.jsonl`. Default: `false`. |

---

## `modules/<name>.toml`

Per-module configuration. All sections are optional.

```toml
# modules/shell.toml

[homebrew]
brewfile = "brew/Brewfile.shell"

[mise]
config = "source/mise.toml"

requires_op = false
```

| Field | Type | Description |
|-------|------|-------------|
| `[homebrew] brewfile` | string | Path to Brewfile, relative to repo root. |
| `[mise] config` | string | Path to mise config file, relative to repo root. |
| `requires_op` | bool | If `true`, skip brew/mise if `op` CLI is unavailable or not signed in. Default: `false`. |

---

## `ai/platforms.toml`

Declares which AI agent platforms are active on this machine.

```toml
active = ["claude-code"]

[platform.claude-code]
skills_dir = "~/.claude/skills"
```

| Field | Description |
|-------|-------------|
| `active` | Array of platform IDs to activate. |
| `[platform.<id>] skills_dir` | Directory where skills for this platform are deployed. |

Run `haven ai discover` to auto-generate this file by scanning installed platforms.

---

## `ai/config.toml`

Optional file that configures the AI skill backend. All fields have defaults;
the file can be omitted entirely.

```toml
# ai/config.toml

[skills]
backend      = "native"   # "native" | "agent-skills" | "akm"
runner       = "skills"   # binary name, path, or array (agent-skills backend only)
timeout_secs = 120        # subprocess timeout in seconds (agent-skills backend only)
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `backend` | string | `"native"` | Skill backend to use. |
| `runner` | string or array | `"skills"` | Binary (or program + args array) used to invoke agent-skills-cli. |
| `timeout_secs` | integer | `120` | Timeout for agent-skills-cli subprocesses. |

### Runner examples

```toml
# Global install (default)
runner = "skills"

# Custom path
runner = "/usr/local/bin/skills"

# Via bunx — no global install required
runner = ["bunx", "agent-skills-cli"]

# Via npx
runner = ["npx", "agent-skills-cli"]
```

Run `haven ai backends` to see which backends are available and which is active.

---

## `ai/skills/<name>/skill.toml`

Declares a single AI skill.

```toml
source    = "gh:anthropics/skills/pdf-processing@v1.0"
platforms = "all"
deploy    = "symlink"
```

| Field | Type | Description |
|-------|------|-------------|
| `source` | string | Skill source: `gh:owner/repo[/subpath][@ref]` or `dir:~/path`. |
| `platforms` | string or array | `"all"`, `"cross-client"`, or array of platform IDs. |
| `deploy` | `"symlink"` or `"copy"` | Deploy method. Default: `"symlink"`. |

---

## `config/ignore`

gitignore-style patterns for files to exclude from `apply`, `status`, and `diff`.

```
# config/ignore

{% if os == "macos" %}
.DS_Store
{% endif %}

.ssh/id_*
.config/*/history
.local/share/some-app/**

!.local/share/some-app/keep-this
```

This file is a Tera template — evaluated at runtime on every command that loads it. If the template fails to render, haven warns and falls back to ignoring nothing.

### Pattern rules

| Syntax | Matches |
|--------|---------|
| `#` at start | Comment (ignored) |
| `*` | Any non-`/` characters |
| `**` | Any characters including `/` |
| `?` | Any single non-`/` character |
| `!` prefix | Negate — un-ignores a previously matched path |
| Pattern with no `/` | Basename only |
| Pattern with `/` | Full path from home root |

---

## `haven.lock`

Auto-generated. Records SHA256 hashes of all fetched external sources.

```toml
# haven.lock — auto-generated, do not edit by hand

[skill."gh:anthropics/skills/pdf-processing@v1.0"]
sha        = "abc123def456..."
fetched_at = "2026-03-21T10:00:00Z"
```

Commit this file alongside your config. Run `haven ai update` to intentionally upgrade a skill and refresh its SHA.
