#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

echo "🎵 Uruchamianie OxiTerm Spotify Control Center..."

# 1. Sprawdzenie pliku .env
if [ ! -f "$SCRIPT_DIR/.env" ]; then
    echo "⚠️ Brak pliku .env! Tworzenie z domyślnym Client ID..."
    cp "$SCRIPT_DIR/.env.example" "$SCRIPT_DIR/.env"
fi

# 2. Uruchomienie backendu w Dockerze (lub venv jako fallback)
if command -v docker &> /dev/null && command -v docker-compose &> /dev/null || docker compose version &> /dev/null; then
    echo "🐳 Uruchamianie backendu Python App Server w Dockerze..."
    docker compose -f "$SCRIPT_DIR/docker-compose.yml" up -d --build
else
    echo "🐍 Docker niedostępny — uruchamianie backendu w środowisku venv..."
    if [ ! -d "$SCRIPT_DIR/venv" ]; then
        python3 -m venv "$SCRIPT_DIR/venv"
        "$SCRIPT_DIR/venv/bin/pip" install -r "$SCRIPT_DIR/requirements.txt"
    fi
    "$SCRIPT_DIR/venv/bin/python3" "$SCRIPT_DIR/app.py" &
    APP_PID=$!
    trap "kill $APP_PID 2>/dev/null || true" EXIT
fi

sleep 2

# 3. Uruchomienie OxiTerm TUI Server
export OXITERM_APP_SERVER="http://localhost:8889/events"
export OXITERM_NO_AUTH="true"
export OXITERM_PORT="2222"
export OXITERM_WEB_PORT="8080"

echo "🚀 Serwer OxiTerm aktywny!"
echo "   - Połączenie SSH: ssh localhost -p 2222"
echo "   - Połączenie Web: http://localhost:8080"
echo "   - Dostęp zdalny:  https://oxiterm.slavekm.pl"

cd "$ROOT_DIR"
cargo run --release --bin oxiterm-cli -- serve "$SCRIPT_DIR/spotify_panel.thtml"
