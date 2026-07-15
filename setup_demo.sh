#!/bin/bash
set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTH_FILE="$PROJECT_ROOT/authorized_keys"

echo "🔧 Konfigurowanie demo OxiTerm..."

# Szukanie klucza publicznego użytkownika
PUB_KEY=""
if [ -f "$HOME/.ssh/id_ed25519.pub" ]; then
    PUB_KEY="$HOME/.ssh/id_ed25519.pub"
elif [ -f "$HOME/.ssh/id_rsa.pub" ]; then
    PUB_KEY="$HOME/.ssh/id_rsa.pub"
fi

if [ -n "$PUB_KEY" ]; then
    echo "✅ Znaleziono klucz publiczny: $PUB_KEY"
    cp "$PUB_KEY" "$AUTH_FILE"
    echo "✅ Klucz skopiowany do $AUTH_FILE"
else
    echo "⚠️  Nie znaleziono klucza publicznego w ~/.ssh/id_ed25519.pub lub ~/.ssh/id_rsa.pub"
    echo "Aby połączyć się przez SSH, musisz dodać swój klucz publiczny do pliku: $AUTH_FILE"
fi

echo ""
echo "🚀 Jak uruchomić demo:"
echo "1. W jednym terminalu uruchom serwer (w trybie --release dla pełnej wydajności):"
echo "   cd $PROJECT_ROOT && OXITERM_PASSWORD=krakow RUST_LOG=info cargo run --release -p oxiterm-server"
echo ""
echo "2. W drugim terminalu połącz się:"
echo "   ssh -p 2222 localhost"
echo ""
echo "💡 Nawigacja w aplikacji:"
echo "   [1] Aktualna pogoda"
echo "   [2] 7-dniowa prognoza"
echo "   [3] Szczegóły"
echo "   [Tab] Przełącz widok"
echo "   [R] Odśwież dane"
echo "   [Q] Wyjdź"
