# Template Variables

All variables available in `.tmpl` files. Run `haven data` to see the actual values on the current machine.

## Built-in variables

| Variable | Example value | Description |
|----------|--------------|-------------|
| `{{ os }}` | `"macos"`, `"linux"` | Operating system |
| `{{ hostname }}` | `"my-laptop"` | Machine hostname |
| `{{ username }}` | `"alice"` | Current user (`$USER`) |
| `{{ home_dir }}` | `"/Users/alice"` | Home directory path |
| `{{ source_dir }}` | `"/Users/alice/.local/share/haven"` | haven repo root path |
| `{{ profile }}` | `"default"`, `"work"` | Active profile name |
| `{{ arch }}` | `"aarch64"`, `"x86_64"` | CPU architecture |

## Functions

| Call | Description |
|------|-------------|
| `{{ get_env(name="VAR") }}` | Value of environment variable `VAR`. Errors if unset. |
| `{{ get_env(name="VAR", default="x") }}` | Value of `VAR`, or `"x"` if unset. |
| `{{ op(path="vault/item/field") }}` | Read a secret from 1Password at apply time. |
| `{{ op(path="op://vault/item/field") }}` | Same with explicit `op://` prefix. |

## Custom data variables

Defined in `[data]` in `haven.toml`:

```toml
[data]
work_email    = "alice@corp.example"
kanata_path   = "/usr/local/bin/kanata"
```

Accessed in templates as `{{ data.<key> }}`:

```
{{ data.work_email }}
{{ data.kanata_path }}
```

## OS values

| OS | `{{ os }}` value |
|----|-----------------|
| macOS | `"macos"` |
| Linux | `"linux"` |

Note: haven uses `"macos"`, not `"darwin"`. This differs from chezmoi.

## Arch values

| Architecture | `{{ arch }}` value |
|-------------|-------------------|
| Apple Silicon | `"aarch64"` |
| Intel Mac / x86_64 Linux | `"x86_64"` |
| 32-bit ARM | `"armv7"` |
| 32-bit x86 | `"i686"` |

## Example: checking all variables

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
arch       = aarch64
profile    = default

data.work_email  = alice@corp.example
data.kanata_path = /usr/local/bin/kanata
```

## Template syntax quick reference

| Construct | Syntax |
|-----------|--------|
| Output variable | `{{ variable }}` |
| If | `{% if condition %}...{% endif %}` |
| If/else | `{% if condition %}...{% else %}...{% endif %}` |
| Elif | `{% if a %}...{% elif b %}...{% else %}...{% endif %}` |
| For loop | `{% for item in list %}...{% endfor %}` |
| Comment | `{# ignored #}` |
| Logical AND | `{% if a and b %}` |
| Logical OR | `{% if a or b %}` |
| Logical NOT | `{% if not a %}` |
| String equals | `{% if os == "macos" %}` |
| String not equal | `{% if os != "linux" %}` |
| Filter | `{{ value \| upper }}` |
| Default filter | `{{ value \| default(value="fallback") }}` |

Full Tera reference: [keats.github.io/tera/docs](https://keats.github.io/tera/docs/)
