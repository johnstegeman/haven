# Installation

## macOS and Linux (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh
```

The installer:

1. Detects your OS and CPU architecture
2. Downloads the matching binary from the [latest GitHub release](https://github.com/johnstegeman/haven/releases)
3. Verifies the SHA256 checksum
4. Installs to `/usr/local/bin` (or `~/.local/bin` if `/usr/local/bin` is not writable)

## Pinning a version

```sh
VERSION=v0.5.0 curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh
```

## Build from source

Requires Rust 1.75+:

```sh
cargo install --git https://github.com/johnstegeman/haven
```

## Supported platforms

| Platform | Architectures |
|----------|---------------|
| macOS | arm64 (Apple Silicon), x86_64 (Intel) |
| Linux | x86_64, aarch64, armv7, i686 |

## Shell completions

After installing, set up tab completions for your shell.

**Fish:**

```sh
haven completions fish > ~/.config/fish/completions/haven.fish
```

**Zsh** — add to `~/.zshrc`:

```sh
source <(haven completions zsh)
```

**Bash** — add to `~/.bashrc`:

```sh
source <(haven completions bash)
```

## Upgrading

```sh
haven upgrade           # upgrade to the latest version
haven upgrade --check   # check without installing
haven upgrade --force   # reinstall even if already on latest
```

## Next steps

- [Quick Start](quickstart.md) — initialize a repo and start tracking files
- [New Machine Setup](new-machine.md) — apply an existing environment to a new machine
