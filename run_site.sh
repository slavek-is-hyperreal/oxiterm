#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPOTIFY_DIR="$SCRIPT_DIR/spotify-app-server"
ENTRY_FILE="$SCRIPT_DIR/examples/index.thtml"

SSH_PORT="${OXITERM_PORT:-2222}"
WEB_PORT="${OXITERM_WEB_PORT:-8087}"

echo "🚀 Uruchamianie OxiTerm Website & Control Center..."

# 0. Git hooks config if present
git -C "$SCRIPT_DIR" config core.hooksPath .githooks 2>/dev/null || true

# 1. Spotify App Server handling
if [ -f "$SPOTIFY_DIR/.env" ]; then
    APP_TOKEN="$(grep "^OXITERM_APP_TOKEN=" "$SPOTIFY_DIR/.env" | cut -d= -f2- || true)"
    if [ -z "$APP_TOKEN" ]; then
        echo "⚠️ WARNING: OXITERM_APP_TOKEN w $SPOTIFY_DIR/.env jest pusty!"
        echo "   Wysyłanie poprawek stanu (POST /sessions/{id}/patch) zwróci błąd 404 (Unauthorized)."
    fi

    if command -v docker &> /dev/null && (command -v docker-compose &> /dev/null || docker compose version &> /dev/null); then
        echo "🐳 Uruchamianie backendu Python App Server w Dockerze..."
        docker compose -f "$SPOTIFY_DIR/docker-compose.yml" up -d --build
    else
        echo "🐍 Docker niedostępny — uruchamianie backendu w środowisku venv..."
        if [ ! -d "$SPOTIFY_DIR/venv" ]; then
            python3 -m venv "$SPOTIFY_DIR/venv"
            "$SPOTIFY_DIR/venv/bin/pip" install -r "$SPOTIFY_DIR/requirements.txt"
        fi
        "$SPOTIFY_DIR/venv/bin/python3" "$SPOTIFY_DIR/app.py" &
        APP_PID=$!
        trap "kill $APP_PID 2>/dev/null || true" EXIT
    fi

    export OXITERM_APP_SERVER="http://localhost:8889/events"
    export OXITERM_APP_TOKEN="$APP_TOKEN"
else
    echo "ℹ️ Brak pliku $SPOTIFY_DIR/.env — strona wystartuje bez podłączonego App Servera Spotify."
fi

# 2. Build TUI Server
echo ">>> Budowanie oxiterm-cli..."
cargo build --release -p oxiterm-cli
BINARY="$SCRIPT_DIR/target/release/oxiterm-cli"

echo ""
echo "========================================="
echo "  OxiTerm Project Website"
echo "========================================="
echo "  Strona główna: $ENTRY_FILE"
echo "  SSH:           ssh localhost -p $SSH_PORT"
echo "  Web:           http://localhost:$WEB_PORT"
echo "  Zdalnie:       https://oxiterm.slavekm.pl"
echo "  Ctrl+C         aby zatrzymać"
echo "========================================="
echo ""

exec "$BINARY" serve "$ENTRY_FILE" \
    --port "$SSH_PORT" \
    --web-port "$WEB_PORT" \
    --no-auth
