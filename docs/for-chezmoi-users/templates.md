# Template Conversion

haven uses [Tera](https://keats.github.io/tera/) (Jinja2-compatible) instead of Go templates. The importer converts most constructs automatically, but complex expressions may need manual attention.

## Variable mapping

| chezmoi / Go | haven / Tera |
|-------------|--------------|
| `{{ .chezmoi.hostname }}` | `{{ hostname }}` |
| `{{ .chezmoi.username }}` | `{{ username }}` |
| `{{ .chezmoi.os }}` | `{{ os }}` |
| `{{ .chezmoi.arch }}` | `{{ arch }}` |
| `{{ .chezmoi.homeDir }}` | `{{ home_dir }}` |
| `{{ .chezmoi.sourceDir }}` | `{{ source_dir }}` |
| `{{ .someCustomVar }}` | `{{ data.someCustomVar }}` |
| `{{ (index . "someVar") }}` | `{{ data.someVar }}` |
| `{{ env "VAR" }}` | `{{ get_env(name="VAR") }}` |

!!! note "Custom data namespacing"
    chezmoi accesses custom data as `.key`. haven namespaces it under `data.key` to avoid collisions with built-in variables. The importer rewrites these automatically and migrates values from `.chezmoidata.*` into `[data]` in `haven.toml`.

!!! note "OS name"
    chezmoi uses `"darwin"` for macOS; haven uses `"macos"`. The importer rewrites these automatically.

## Control flow

| chezmoi / Go | haven / Tera |
|-------------|--------------|
| `{{- if eq .chezmoi.os "darwin" -}}` | `{% if os == "macos" %}` |
| `{{- if eq .chezmoi.os "linux" -}}` | `{% if os == "linux" %}` |
| `{{- else if eq .chezmoi.os "linux" -}}` | `{% elif os == "linux" %}` |
| `{{- else -}}` | `{% else %}` |
| `{{- end -}}` | `{% endif %}` |
| `{{ if .someVar }}` | `{% if data.someVar %}` |
| `{{ range .someList }}` | `{% for item in data.someList %}` |
| `{{ end }}` (range) | `{% endfor %}` |

## Comments

| chezmoi / Go | haven / Tera |
|-------------|--------------|
| `{{/* comment */}}` | `{# comment #}` |

## 1Password integration

| chezmoi | haven |
|---------|-------|
| `{{ onepasswordField "Personal" "GitHub" "token" }}` | `{{ op(path="Personal/GitHub/token") }}` |
| `{{ onepassword "op://Personal/GitHub/token" }}` | `{{ op(path="op://Personal/GitHub/token") }}` |

## Before and after example

**chezmoi (Go templates):**

```
[user]
  email = {{ if eq .chezmoi.os "darwin" }}{{ .work_email }}{{ else }}{{ .personal_email }}{{ end }}
  name = {{ .chezmoi.username }}

[core]
  editor = {{ env "EDITOR" }}
```

**haven (Tera):**

```
[user]
{% if os == "macos" %}
  email = {{ data.work_email }}
{% else %}
  email = {{ data.personal_email }}
{% endif %}
  name = {{ username }}

[core]
  editor = {{ get_env(name="EDITOR") }}
```

## Constructs that need manual conversion

The importer marks unconvertible constructs with `# haven: TODO`:

```
# haven: TODO: complex pipeline: {{ .someList | join "," }}
{{ .someList | join "," }}
```

Find them after importing:

```sh
grep -r "haven: TODO" ~/.local/share/haven/source/
```

### Common manual conversions

**String join:**
```
# Go:   {{ .list | join "," }}
# Tera: {{ data.list | join(sep=",") }}
```

**Default value:**
```
# Go:   {{ .value | default "fallback" }}
# Tera: {{ data.value | default(value="fallback") }}
```

**String contains:**
```
# Go:   {{ contains .str "substr" }}
# Tera: {% if str is containing("substr") %}
```

**Custom functions:** Go template custom functions registered by chezmoi (like `bitwarden`, `lastpass`, `vault`) have no Tera equivalent. Replace these with `{{ op(path="...") }}` calls if you use 1Password, or migrate the secret to an environment variable.

## Checking variables in scope

After migration, run:

```sh
haven data
```

To see all variables available in templates on this machine:

```
os         = macos
hostname   = my-laptop
username   = alice
home_dir   = /Users/alice
source_dir = /Users/alice/.local/share/haven

data.work_email  = alice@corp.example
data.kanata_path = /usr/local/bin/kanata
```
