#!/bin/bash
# demo.sh — uruchamia OxiTerm z menu examples/hello.thtml
# SSH:  ssh -p 2222 localhost
# Web:  http://localhost:8087

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLES_DIR="$SCRIPT_DIR/examples"
ENTRY_FILE="$EXAMPLES_DIR/hello.thtml"

SSH_PORT="${OXITERM_PORT:-2222}"
WEB_PORT="${OXITERM_WEB_PORT:-8087}"

# --- build ---
echo ">>> Budowanie oxiterm-cli..."
cargo build -p oxiterm-cli --release 2>&1
BINARY="$SCRIPT_DIR/target/release/oxiterm-cli"

if [ ! -f "$BINARY" ]; then
    echo "BŁĄD: nie znaleziono binarki $BINARY" >&2
    exit 1
fi

if [ ! -f "$ENTRY_FILE" ]; then
    echo "BŁĄD: nie znaleziono $ENTRY_FILE" >&2
    exit 1
fi

echo ""
echo "========================================="
echo "  OxiTerm Demo"
echo "========================================="
echo "  Plik:    $ENTRY_FILE"
echo "  SSH:     ssh -p $SSH_PORT localhost"
echo "  Web:     http://localhost:$WEB_PORT"
echo "  Ctrl+C   aby zatrzymać"
echo "========================================="
echo ""

exec "$BINARY" serve "$ENTRY_FILE" \
    --port "$SSH_PORT" \
    --web-port "$WEB_PORT" \
    --no-auth
