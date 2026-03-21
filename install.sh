#!/bin/sh
# dfiles installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/jstegeman/dfiles/main/install.sh | sh
#
# The installer:
#   1. Detects your OS and CPU architecture
#   2. Downloads the matching binary from the latest GitHub Release
#   3. Verifies the SHA256 checksum
#   4. Installs to /usr/local/bin (or ~/.local/bin if /usr/local/bin is not writable)
#
set -eu

REPO="johnstegeman/dfiles"
BINARY="dfiles"

# ─── helpers ──────────────────────────────────────────────────────────────────

info()  { printf '\033[1;34minfo\033[0m  %s\n' "$*" >&2; }
ok()    { printf '\033[1;32m ok \033[0m  %s\n' "$*" >&2; }
warn()  { printf '\033[1;33mwarn\033[0m  %s\n' "$*" >&2; }
die()   { printf '\033[1;31merror\033[0m %s\n' "$*" >&2; exit 1; }

need_cmd() {
    if ! command -v "$1" > /dev/null 2>&1; then
        die "Required command not found: $1"
    fi
}

download() {
    url="$1"
    dest="$2"
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL --retry 3 -o "$dest" "$url"
    elif command -v wget > /dev/null 2>&1; then
        wget -qO "$dest" "$url"
    else
        die "Neither curl nor wget found. Install one and retry."
    fi
}

# ─── OS detection ─────────────────────────────────────────────────────────────

detect_os() {
    os="$(uname -s)"
    case "$os" in
        Darwin) echo "darwin" ;;
        Linux)  echo "linux"  ;;
        *)      die "Unsupported OS: $os. dfiles supports macOS and Linux." ;;
    esac
}

# ─── Architecture detection ───────────────────────────────────────────────────

detect_arch() {
    arch="$(uname -m)"
    case "$arch" in
        x86_64)         echo "x86_64"   ;;
        amd64)          echo "x86_64"   ;;
        aarch64)        echo "aarch64"  ;;
        arm64)          echo "aarch64"  ;;
        i686)           echo "i686"     ;;
        i386)           echo "i686"     ;;
        armv7l)         echo "armv7"    ;;
        *)              die "Unsupported architecture: $arch. Supported: x86_64, aarch64, i686, armv7." ;;
    esac
}

# ─── Target triple ────────────────────────────────────────────────────────────

build_target() {
    os="$1"
    arch="$2"
    case "${os}-${arch}" in
        darwin-x86_64)   echo "x86_64-apple-darwin"              ;;
        darwin-aarch64)  echo "aarch64-apple-darwin"             ;;
        linux-x86_64)    echo "x86_64-unknown-linux-musl"        ;;
        linux-i686)      echo "i686-unknown-linux-musl"          ;;
        linux-aarch64)   echo "aarch64-unknown-linux-musl"       ;;
        linux-armv7)     echo "armv7-unknown-linux-musleabihf"   ;;
        *)               die "No release available for ${os}/${arch}." ;;
    esac
}

# ─── Latest release version ───────────────────────────────────────────────────

latest_version() {
    api_url="https://api.github.com/repos/${REPO}/releases/latest"
    info "Fetching latest release from GitHub..."

    tmpfile="$(mktemp)"
    # Capture HTTP status to give a better error message on rate-limit
    if command -v curl > /dev/null 2>&1; then
        http_code="$(curl -fsSL -w "%{http_code}" -o "$tmpfile" "$api_url" 2>/dev/null || true)"
    else
        wget -qO "$tmpfile" "$api_url" 2>/dev/null || true
        http_code="200"  # wget doesn't easily give status; failure shows as empty file
    fi

    if [ "${http_code:-}" = "403" ] || [ "${http_code:-}" = "429" ]; then
        rm -f "$tmpfile"
        die "GitHub API rate limit reached. Wait a minute and try again, or set VERSION env var:\n  VERSION=v0.1.0 sh install.sh"
    fi

    version="$(grep '"tag_name"' "$tmpfile" | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')"
    rm -f "$tmpfile"

    if [ -z "$version" ]; then
        die "Could not determine latest version. Check https://github.com/${REPO}/releases"
    fi
    echo "$version"
}

# ─── SHA256 verification ──────────────────────────────────────────────────────

verify_checksum() {
    archive="$1"
    sums_file="$2"

    archive_name="$(basename "$archive")"
    expected="$(grep " ${archive_name}" "$sums_file" | awk '{print $1}')"

    if [ -z "$expected" ]; then
        die "No checksum found for ${archive_name} in SHA256SUMS"
    fi

    if command -v sha256sum > /dev/null 2>&1; then
        actual="$(sha256sum "$archive" | awk '{print $1}')"
    elif command -v shasum > /dev/null 2>&1; then
        actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
    else
        warn "Neither sha256sum nor shasum found — skipping checksum verification."
        return 0
    fi

    if [ "$actual" != "$expected" ]; then
        die "Checksum mismatch for ${archive_name}:\n  expected: ${expected}\n  actual:   ${actual}"
    fi
}

# ─── Installation directory ───────────────────────────────────────────────────

choose_install_dir() {
    if [ -w "/usr/local/bin" ]; then
        echo "/usr/local/bin"
    elif [ -d "/usr/local/bin" ]; then
        # Exists but not writable — try sudo later, for now return it
        echo "/usr/local/bin"
    else
        echo "${HOME}/.local/bin"
    fi
}

# ─── Main ─────────────────────────────────────────────────────────────────────

main() {
    OS="$(detect_os)"
    ARCH="$(detect_arch)"
    TARGET="$(build_target "$OS" "$ARCH")"

    VERSION="${VERSION:-$(latest_version)}"
    info "Installing dfiles ${VERSION} for ${OS}/${ARCH} (${TARGET})"

    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
    ARCHIVE="dfiles-${VERSION}-${TARGET}.tar.gz"
    SUMS="dfiles-${VERSION}-SHA256SUMS"

    # Download to a temp directory
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT

    info "Downloading ${ARCHIVE}..."
    download "${BASE_URL}/${ARCHIVE}" "${TMPDIR}/${ARCHIVE}"

    info "Downloading SHA256SUMS..."
    download "${BASE_URL}/${SUMS}" "${TMPDIR}/${SUMS}"

    info "Verifying checksum..."
    verify_checksum "${TMPDIR}/${ARCHIVE}" "${TMPDIR}/${SUMS}"
    ok "Checksum verified."

    info "Extracting binary..."
    tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}"

    INSTALL_DIR="$(choose_install_dir)"
    mkdir -p "$INSTALL_DIR"

    DEST="${INSTALL_DIR}/${BINARY}"
    if [ -w "$INSTALL_DIR" ]; then
        cp "${TMPDIR}/${BINARY}" "$DEST"
    else
        info "Installing to ${INSTALL_DIR} (requires sudo)..."
        sudo cp "${TMPDIR}/${BINARY}" "$DEST"
    fi
    chmod 755 "$DEST"

    # PATH warning for ~/.local/bin
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            warn "${INSTALL_DIR} is not in your PATH."
            warn "Add this to your shell profile:"
            warn "  export PATH=\"\${HOME}/.local/bin:\${PATH}\""
            ;;
    esac

    # Verify the installed binary runs
    if ! "$DEST" --version > /dev/null 2>&1; then
        die "Installed binary failed to run. Please report this at https://github.com/${REPO}/issues"
    fi

    installed_version="$("$DEST" --version 2>&1 | head -1)"
    ok "Installed: ${DEST}"
    ok "${installed_version}"
}

main "$@"
