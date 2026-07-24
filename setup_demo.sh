#!/bin/bash
set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTH_FILE="$PROJECT_ROOT/authorized_keys"

echo "🔧 Konfigurowanie środowiska deweloperskiego OxiTerm..."

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
echo "🚀 Jak uruchomić przykładowe demo:"
echo "1. Uruchom serwer demo hello.thtml:"
echo "   ./demo.sh"
echo ""
echo "2. W drugim terminalu połącz się przez SSH:"
echo "   ssh -p 2222 localhost"
echo "   lub otwórz w przeglądarce: http://localhost:8080"
echo ""
echo "💡 Nawigacja w aplikacji showcase (examples/hello.thtml):"
echo "   [Tab] / [Strzałki]   Przełączanie fokusu elementów"
echo "   [Enter]              Aktywacja przycisku lub zmiana zakładki"
echo "   [PgUp] / [PgDn]      Przewijanie strony"
echo "   [Q]                  Wyjście z sesji"
