#!/usr/bin/env bash
set -euo pipefail

# Full web build + deploy pipeline using Trunk.
#
# Pipeline:
#   1. Sync static files:  crates/channels/static/ → crates/web/
#   2. Trunk build:        crates/web/ → crates/web/dist/
#   3. Deploy:             crates/web/dist/ → ~/.openpista/web/
#   4. (optional) Restart: kill + relaunch web server
#
# Usage:
#   ./scripts/build-web.sh            # build + deploy
#   ./scripts/build-web.sh --restart  # build + deploy + restart server
#
# Prerequisites:
#   rustup target add wasm32-unknown-unknown
#   cargo install trunk --locked

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WEB_CRATE="$PROJECT_ROOT/crates/web"
STATIC_SRC="$PROJECT_ROOT/crates/channels/static"
DEPLOY_DIR="$HOME/.openpista/web"
PID_FILE="$HOME/.openpista/openpista.pid"
BINARY="$PROJECT_ROOT/target/release/openpista"

RESTART=false
for arg in "$@"; do
  case "$arg" in
    --restart) RESTART=true ;;
  esac
done

echo "=== openpista web build pipeline ==="
echo ""

# ── Step 1: Sync static files → crates/web/ ──
echo "[1/4] Syncing static files..."
cp "$STATIC_SRC/app.js"    "$WEB_CRATE/app.js"
cp "$STATIC_SRC/style.css" "$WEB_CRATE/style.css"
echo "  ✓ app.js + style.css → crates/web/"

# ── Step 2: Validate JS syntax ──
echo "[2/4] Validating JavaScript..."
if command -v node &>/dev/null; then
  node -c "$WEB_CRATE/app.js"
  echo "  ✓ app.js syntax OK"
else
  echo "  ⚠ node not found, skipping JS validation"
fi

# ── Step 3: Trunk build ──
echo "[3/4] Running trunk build --release..."
cd "$WEB_CRATE"
trunk build --release
echo "  ✓ Trunk build complete → crates/web/dist/"

# ── Step 4: Deploy to ~/.openpista/web/ ──
echo "[4/4] Deploying to $DEPLOY_DIR..."
mkdir -p "$DEPLOY_DIR"
cp -r "$WEB_CRATE/dist/"* "$DEPLOY_DIR/"
echo "  ✓ Deployed to $DEPLOY_DIR"

echo ""
echo "=== Build + deploy complete ==="

# ── Optional: Restart server ──
if [ "$RESTART" = true ]; then
  echo ""
  echo "Restarting web server..."
  if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    kill "$PID" 2>/dev/null || true
    sleep 1
  fi
  if [ -x "$BINARY" ]; then
    "$BINARY" web start &
    sleep 2
    echo "  ✓ Server restarted (PID: $(cat "$PID_FILE" 2>/dev/null || echo '?'))"
  else
    echo "  ✗ Binary not found at $BINARY — run 'cargo build --release' first"
    exit 1
  fi
fi
