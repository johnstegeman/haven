# Environment Variables

Environment variables for configuring haven's behavior without editing config files.

| Variable | Default | Description |
|----------|---------|-------------|
| `HAVEN_DIR` | `~/.local/share/haven` | haven repo root directory. Equivalent to `--dir` flag. |
| `HAVEN_CLAUDE_DIR` | `~/.claude` | Claude Code directory (skills, CLAUDE.md). |
| `HAVEN_TELEMETRY` | unset | Set to `1` to enable telemetry, `0` to force-disable (overrides `haven.toml`). |
| `HAVEN_VCS` | unset | Set to `git` or `jj` to override the VCS backend. |

## Priority order

When the same setting can be configured in multiple places, haven uses this priority (first match wins):

**VCS backend:**

1. `--vcs` CLI flag
2. `HAVEN_VCS` environment variable
3. `vcs.backend` in `haven.toml`
4. Interactive detection (if jj is on PATH and nothing is configured, haven prompts)
5. Default: `git`

**Repo directory:**

1. `--dir` CLI flag
2. `HAVEN_DIR` environment variable
3. Default: `~/.local/share/haven`

**Telemetry:**

1. `HAVEN_TELEMETRY=0` (force-disable)
2. `HAVEN_TELEMETRY=1` (force-enable)
3. `[telemetry] enabled` in `haven.toml`
4. Default: disabled

## Examples

```sh
# Use a custom repo location for this session
HAVEN_DIR=~/work-env haven apply --profile work

# Use jj for this command only
HAVEN_VCS=jj haven init gh:alice/dotfiles

# Enable telemetry for one command
HAVEN_TELEMETRY=1 haven apply
```
