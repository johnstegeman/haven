# haven development tasks
#
# Usage: just <recipe>
# Requires: just (https://github.com/casey/just)

# List available recipes
default:
    @just --list

# Run all tests
test:
    cargo test

# Run tests with output
test-verbose:
    cargo test -- --nocapture

# Build a release binary for the current platform
build:
    cargo build --release

# Run clippy
lint:
    cargo clippy -- -D warnings

# Build and deploy docs to GitHub Pages
docs:
    mkdocs gh-deploy --force

# Preview docs locally
docs-serve:
    mkdocs serve

# Cut a release: bump version, commit, tag
# Usage: just release 0.2.0
release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail

    # Validate semver format (no 'v' prefix expected here)
    if ! echo "{{VERSION}}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "error: VERSION must be bare semver without 'v' prefix"
        echo "  example: just release 0.2.0"
        exit 1
    fi

    # Ensure working copy is clean (jj)
    if jj st 2>/dev/null | grep -q "^Working copy changes:"; then
        echo "error: working copy has uncommitted changes. Describe and squash first."
        exit 1
    fi

    echo "Bumping Cargo.toml to {{VERSION}}..."
    sed -i.bak '3s/version = "[^"]*"/version = "{{VERSION}}"/' Cargo.toml
    rm -f Cargo.toml.bak

    echo "Updating Cargo.lock..."
    cargo check --quiet 2>&1

    echo "Creating jj commit and git tag..."
    jj new
    jj desc -m "chore: release v{{VERSION}}"
    # Absorb the Cargo.toml and Cargo.lock changes into the new commit
    # (they were auto-snapshotted by jj when we ran cargo check)

    # Create the git tag pointing at the current HEAD
    jj git export 2>/dev/null || true
    git tag "v{{VERSION}}"

    echo ""
    echo "Version bumped to {{VERSION}} and tagged v{{VERSION}}."
    echo ""
    echo "Push with:"
    echo "  jj git push -b main"
    echo "  git push origin v{{VERSION}}"
