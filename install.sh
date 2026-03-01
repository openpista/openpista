#!/usr/bin/env bash
# openpista installer — detects OS/arch and downloads the latest release binary.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/openpista/openpista/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/openpista/openpista/main/install.sh | bash -s -- --prefix ~/.local

set -euo pipefail

REPO="openpista/openpista"
BIN_NAME="openpista"

# ─── helpers ────────────────────────────────────────────────────────
info()  { printf '\033[1;34m→\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m✔\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m✘\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || err "'$1' is required but not found"; }

# ─── parse args ─────────────────────────────────────────────────────
PREFIX="/usr/local/bin"
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX="$2"; shift 2 ;;
    --prefix=*) PREFIX="${1#*=}"; shift ;;
    *) err "unknown option: $1" ;;
  esac
done

# ─── detect platform ───────────────────────────────────────────────
detect_target() {
  local os arch libc target

  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}" in
    Linux)
      # detect musl vs glibc
      if ldd --version 2>&1 | grep -qi musl; then
        libc="musl"
      else
        libc="gnu"
      fi
      case "${arch}" in
        x86_64)  target="x86_64-unknown-linux-${libc}" ;;
        aarch64|arm64) target="aarch64-unknown-linux-${libc}" ;;
        *) err "unsupported architecture: ${arch}" ;;
      esac
      ;;
    Darwin)
      case "${arch}" in
        arm64|aarch64) target="aarch64-apple-darwin" ;;
        *) err "unsupported macOS architecture: ${arch} (only Apple Silicon is supported)" ;;
      esac
      ;;
    *) err "unsupported OS: ${os}" ;;
  esac

  echo "${target}"
}

# ─── main ───────────────────────────────────────────────────────────
main() {
  need curl
  need tar

  local target
  target="$(detect_target)"
  info "Detected target: ${target}"

  local url="https://github.com/${REPO}/releases/latest/download/${BIN_NAME}-${target}.tar.gz"
  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "${tmpdir}"' EXIT

  info "Downloading ${url}"
  curl -fsSL "${url}" -o "${tmpdir}/release.tar.gz" \
    || err "download failed — is there a release for ${target}?"

  tar -xzf "${tmpdir}/release.tar.gz" -C "${tmpdir}"

  # find the binary inside the extracted directory
  local bin
  bin="$(find "${tmpdir}" -name "${BIN_NAME}" -type f | head -1)"
  [ -n "${bin}" ] || err "binary not found in archive"

  chmod +x "${bin}"

  # install
  mkdir -p "${PREFIX}"
  if [ -w "${PREFIX}" ]; then
    mv "${bin}" "${PREFIX}/${BIN_NAME}"
  else
    info "Writing to ${PREFIX} requires sudo"
    sudo mv "${bin}" "${PREFIX}/${BIN_NAME}"
  fi

  ok "Installed ${BIN_NAME} to ${PREFIX}/${BIN_NAME}"
  info "Run 'openpista auth login' to get started"
}

main "$@"
