# Templates & Secrets

## Templates

Source files with a `.tmpl` suffix are rendered through the [Tera](https://keats.github.io/tera/) template engine (Jinja2-compatible syntax) before being written to their destination. The `.tmpl` suffix is stripped from the destination filename.

```
source/dot_gitconfig.tmpl  →  ~/.gitconfig  (rendered before writing)
```

Files without `.tmpl` are **never** rendered — curly braces in shell scripts, Makefiles, and similar files are left untouched.

## Available variables

| Variable | Value |
|----------|-------|
| `{{ os }}` | `"macos"`, `"linux"`, or the OS name |
| `{{ hostname }}` | Machine hostname |
| `{{ username }}` | Current user (`$USER`) |
| `{{ home_dir }}` | Home directory path |
| `{{ source_dir }}` | haven repo root path |
| `{{ profile }}` | Active profile name |
| `{{ arch }}` | CPU architecture (e.g. `"aarch64"`, `"x86_64"`) |
| `{{ get_env(name="VAR") }}` | Value of environment variable `VAR` |
| `{{ get_env(name="VAR", default="fallback") }}` | With fallback if unset |
| `{{ data.<key> }}` | Custom variables from `[data]` in `haven.toml` |

Run `haven data` to see all variables in scope for the current machine.

## OS-conditional config

```
# source/dot_gitconfig.tmpl
[core]
{% if os == "macos" %}
  editor = /opt/homebrew/bin/nvim
{% else %}
  editor = /usr/bin/nvim
{% endif %}
```

!!! note "OS name"
    haven uses `"macos"` (not `"darwin"`) for macOS. If you're migrating from chezmoi, the importer rewrites these automatically.

## Profile-conditional config

```
# source/dot_zshrc.tmpl
export PATH="$HOME/.local/bin:$PATH"
{% if profile == "work" %}
source ~/.work-aliases
export CORP_PROXY=http://proxy.corp.example:8080
{% endif %}
```

## Hostname-specific config

```
# source/dot_zshrc.tmpl
{% if hostname == "my-work-laptop" %}
export AWS_PROFILE=work
{% elif hostname == "my-home-mac" %}
export AWS_PROFILE=personal
{% endif %}
```

## Environment variable injection

```
# source/dot_config/tool/config.tmpl
api_base = {{ get_env(name="API_BASE", default="https://api.example.com") }}
```

## Custom data variables

Define machine-specific variables in `haven.toml`:

```toml
[data]
work_email    = "alice@corp.example"
kanata_path   = "/usr/local/bin/kanata"
homebrew_path = "/opt/homebrew"
```

Use them in any `.tmpl` file:

```
# source/dot_gitconfig.tmpl
[user]
  email = {{ data.work_email }}
```

## Tera template syntax quick reference

| Construct | Syntax |
|-----------|--------|
| Variable | `{{ variable }}` |
| If/else | `{% if condition %}...{% elif other %}...{% else %}...{% endif %}` |
| For loop | `{% for item in list %}...{% endfor %}` |
| Comment | `{# this is a comment #}` |
| String comparison | `{% if os == "macos" %}` |
| Logical operators | `and`, `or`, `not` |
| Filters | `{{ variable \| upper }}`, `{{ variable \| default(value="x") }}` |

Full Tera documentation: [keats.github.io/tera/docs](https://keats.github.io/tera/docs/)

---

## 1Password integration

haven can read secrets from 1Password at apply time and render them directly into destination files, without ever storing them in the repo or on disk.

### Prerequisites

1. Install the `op` CLI: [developer.1password.com/docs/cli/get-started](https://developer.1password.com/docs/cli/get-started/)
2. Sign in: `op signin`

### Usage in templates

```
# source/dot_config/gh/hosts.yml.tmpl
github.com:
  user: alice
  oauth_token: {{ op(path="Personal/GitHub/token") }}
```

The full `op://` URI format also works:

```
oauth_token: {{ op(path="op://Personal/GitHub/oauth_token") }}
```

If you omit the `op://` prefix, haven adds it automatically. The path format is `vault/item/field`.

### Module guard

Mark modules that use `op()` with `requires_op = true`:

```toml
# modules/secrets.toml
requires_op = true
```

If `op` is not installed or the user is not signed in, the module is skipped with a warning rather than failing hard. All other modules are applied normally.

### Example: injecting SSH keys via 1Password

```
# source/private_dot_ssh/id_ed25519.tmpl
{{ op(path="op://Personal/SSH Key/private key") }}
```

The destination `~/.ssh/id_ed25519` is written with the private key content fetched live from 1Password. Nothing is stored in the repo.

---

## Checking template variables

```sh
haven data
```

Output:

```
os         = macos
hostname   = my-laptop
username   = alice
home_dir   = /Users/alice
source_dir = /Users/alice/.local/share/haven

data.work_email  = alice@corp.example
data.kanata_path = /usr/local/bin/kanata
```
