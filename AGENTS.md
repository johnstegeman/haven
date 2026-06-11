# Haven — Agent Conventions

## Feedback Instructions

### BUILD COMMANDS
```
cargo build
```

### TEST COMMANDS
```
cargo test
cargo test --test integration
```

### LINT COMMANDS
```
cargo clippy -- -D warnings
```

### FORMAT COMMANDS
```
cargo fmt
```

## Project Conventions

- Rust 2021 edition, binary crate (`haven`)
- No comments that restate what code does — only non-obvious invariants, workarounds, or hidden constraints
- Use `anyhow::Result` for fallible functions
- Tests: unit tests live in `#[cfg(test)] mod tests` at the bottom of each file; integration tests in `tests/integration.rs`
- VCS: this repo uses jj (Jujutsu) colocated with git — use `jj` for all VCS operations
