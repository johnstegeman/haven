# Security Scanning

haven has built-in secret detection to help you avoid accidentally committing credentials, private keys, or sensitive configuration files to your dotfile repo.

## Running a scan

```sh
haven security-scan            # scan all tracked files
haven security-scan --entropy  # also flag high-entropy strings
```

Exits `0` when clean, `1` when findings are reported — suitable for CI or pre-push hooks.

## What is checked

| Check | Examples flagged |
|-------|-----------------|
| **Filename patterns** | `.env`, `id_rsa`, `credentials`, `.pem`, `.key`, `.p12`, `secrets` |
| **Path patterns** | `~/.aws/credentials`, `~/.kube/**`, `~/.ssh/**`, `~/.config/gh/hosts.yml`, `~/.docker/config.json`, `~/.gnupg/**` |
| **Content patterns** | GitHub tokens (`ghp_`, `ghs_`, `github_pat_`), AWS keys (`AKIA…`), PEM private keys, OpenAI keys (`sk-…`), Anthropic keys (`sk-ant-…`), generic `password =` / `secret =` assignments |
| **High-entropy strings** (opt-in) | Random-looking tokens ≥16 chars with Shannon entropy >4.5 bits/char |

High-entropy detection is opt-in (`--entropy`) because it can produce false positives on base64-encoded data, hashes, and other non-secret strings.

## Suppressing false positives

Some files are intentionally tracked even though they look sensitive. Add them to `[security] allow` in `haven.toml`:

```toml
[security]
allow = [
  "~/.config/gh/hosts.yml",     # intentionally tracked — personal access token
  "~/.config/gcloud/**",        # managed by gcloud CLI; not a raw secret
  "~/.config/karabiner/**",     # keyboard config, no secrets
]
```

Patterns follow the same glob syntax as `config/ignore`:

| Pattern | Matches |
|---------|---------|
| `~/.config/gh/hosts.yml` | Exact file |
| `~/.config/gcloud/**` | Everything under `~/.config/gcloud/` |
| `*.example` | Any file with `.example` extension (basename only) |

## Integration with `haven add`

When you run `haven add`, haven scans the file before saving it. If sensitive patterns match, you're prompted:

```
warning: ~/.env may contain sensitive content (1 pattern(s) found).
  · Generic secret assignment (MEDIUM)
Track it anyway? [y/N]
```

Declining removes the file from `source/` immediately — no partial state is left. Files in `[security] allow` bypass this prompt.

## Using in CI

Add to your CI pipeline or as a pre-push hook:

```sh
# pre-push hook: .git/hooks/pre-push
#!/bin/sh
haven security-scan
```

Or in CI:

```yaml
# .github/workflows/ci.yml
- name: Scan for secrets
  run: haven security-scan
```

`haven security-scan` exits 1 on findings, blocking the push or failing the CI job.

## Handling 1Password and other secrets

The recommended approach is to **never store secrets in the repo** — use 1Password templates instead:

```
# source/dot_config/gh/hosts.yml.tmpl
github.com:
  user: alice
  oauth_token: {{ op(path="Personal/GitHub/token") }}
```

The destination file is written with the secret fetched live at apply time. Nothing sensitive is committed. See [Templates & Secrets](templates.md) for details.

## Supply chain protection for skills

haven also protects against supply chain attacks in AI skills. Every `gh:` skill source is pinned by SHA256 in `haven.lock`. A mismatch between the fetched content and the recorded SHA is a hard error:

```sh
# Intentionally upgrade a skill (clears and re-pins the SHA)
haven ai update pdf-processing
```

See [AI Skills](ai-skills.md) for details on the lock file.
