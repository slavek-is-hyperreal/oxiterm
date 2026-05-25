# Przewodnik: Tworzenie aplikacji z OxiTerm

OxiTerm to framework do tworzenia aplikacji TUI (Terminal User Interface) renderowanych po stronie serwera (Server-Side Rendered). Interfejs opisujesz deklaratywnie za pomocą języka znaczników **THTML** oraz stylizujesz za pomocą **TCSS**. Serwer OxiTerm parsuje te pliki, oblicza układ (layout) za pomocą Flexboxa, a następnie przesyła minimalne instrukcje ANSI (diffy) do klienta po SSH lub WebSocket. Klient potrzebuje jedynie zwykłego terminala — nie musi instalować żadnego dedykowanego oprogramowania.

---

## 1. Instalacja

Do zbudowania OxiTerm ze źródeł wymagane jest środowisko Rust. Jeśli go nie posiadasz, zainstaluj je ze strony: [https://rustup.rs](https://rustup.rs)

Następnie sklonuj repozytorium i zbuduj projekt:
```bash
git clone https://github.com/slavek-is-hyperreal/oxiterm && cd oxiterm
cargo build --release
```
Skompilowany plik wykonywalny znajdziesz w: `target/release/oxiterm`

---

## 2. Szybki start (Pierwsza aplikacja)

Utwórz plik o nazwie `hello.thtml` z następującą zawartością:

```html
<box style="flex-direction: column; width: 80; height: 24; bg: #0f172a;">
  <box style="height: 3; bg: #1e293b; align-items: center;
              padding-left: 2; border-style: single; border-color: #334155;">
    <text style="fg: #38bdf8; height: 1;">Witaj w OxiTerm!</text>
  </box>
  <box style="padding: 2;">
    <text style="fg: #e2e8f0; height: 1;">To jest aplikacja TUI renderowana po stronie serwera.</text>
  </box>
</box>
```

Uruchom serwer w trybie deweloperskim (bez autoryzacji):
```bash
oxiterm serve hello.thtml --port 2222 --no-auth
```

Teraz możesz połączyć się z aplikacją z dowolnego terminala za pomocą SSH:
```bash
ssh localhost -p 2222
```

---

## 3. Hot Reload

OxiTerm automatycznie monitoruje serwowany plik `.thtml`. Po edycji i zapisaniu pliku na dysku, wszystkie aktywne sesje połączeń SSH i Web zostaną **natychmiast zaktualizowane** bez konieczności ponownego łączenia. Co ważne, stan aplikacji (zapisany w `StateManager`) jest w pełni zachowywany przy przeładowaniu układu!

---

## 4. Dodawanie Stanu i Akcji

Stanem i interakcją zarządzasz za pomocą atrybutów `bind-state` oraz `event-htmx`:

```html
<box style="flex-direction: column; width: 80; height: 24; bg: #0f172a;">
  <!-- bind-state wiąże zawartość tekstową z kluczem stanu "n" -->
  <text bind-state="n" style="fg: #fbbf24; height: 1; margin: 2;">0</text>
  
  <box style="flex-direction: row; padding-left: 2;">
    <!-- event-htmx definiuje akcję po kliknięciu lub zatwierdzeniu Enterem -->
    <box style="border-style: single; border-color: #f87171; padding: 1; height: 3;
               align-items: center; margin-right: 1;" event-htmx="dec:n">
      <text style="fg: #f87171; height: 1;">−</text>
    </box>
    <box style="border-style: single; border-color: #4ade80; padding: 1; height: 3;
               align-items: center;" event-htmx="inc:n">
      <text style="fg: #4ade80; height: 1;">+</text>
    </box>
  </box>
</box>
```

---

## 5. Klasy CSS

Aby uniknąć powtarzania styli inline, możesz zdefiniować blok `<style>` zawierający reguły TCSS:

```html
<style>
  .card  { border-style: rounded; padding: 1; flex-direction: column; }
  .blue  { border-color: #38bdf8; }
  .muted { fg: #64748b; height: 1; }
</style>

<box class="card blue" style="height: 8;">
  <text style="fg: #38bdf8; height: 1;">Niebieska karta</text>
  <text class="muted">Przykładowa zawartość karty.</text>
</box>
```

---

## 6. Zakładki z użyciem `bind-show`

Atrybut `bind-show` pozwala na warunkowe ukrywanie lub pokazywanie elementów na podstawie stanu:

```html
<box style="flex-direction: row; height: 3;">
  <box event-htmx="set:tab=info" style="border-style: single; padding: 1; height: 3;">
    <text style="height: 1;">Info</text>
  </box>
  <box event-htmx="set:tab=logs" style="border-style: single; padding: 1; height: 3;">
    <text style="height: 1;">Logi</text>
  </box>
</box>

<!-- tab=false oznacza, że element jest widoczny, gdy klucz "tab" jest nieobecny w stanie (domyślnie widoczny) -->
<box bind-show="tab=false" style="padding: 2;">
  <text style="height: 1;">Zawartość zakładki Info.</text>
</box>

<box bind-show="tab=logs" style="padding: 2;">
  <text style="height: 1;">Zawartość zakładki Logi.</text>
</box>
```

---

## 7. Pola tekstowe (Input)

Pole `<input>` pozwala na pobieranie danych wpisywanych przez użytkownika:

```html
<input bind-value="query" placeholder="Szukaj..."
       style="height: 1; border-style: single; border-color: #38bdf8;"/>
<text bind-state="query" style="fg: #94a3b8; height: 1;"/>
```

* Użyj klawisza `Tab`, aby ustawić focus na elemencie `<input>`.
* Zacznij pisać — bufor wejściowy Predictive Echo natychmiast zaktualizuje widok (lokalnie).
* Klawisz `Backspace` usuwa ostatni znak.
* Klawisz `Enter` zatwierdza wpisany tekst i zapisuje go w stanie pod kluczem zdefiniowanym w `bind-value` (np. `"query"`), a także uruchamia akcję `event-htmx`, jeśli została podana.

---

## 8. Obrazy i Media

OxiTerm umożliwia bezpośrednie osadzanie grafiki wektorowej, animacji oraz plików wideo:

```html
<img src="logo.svg"    alt="Logo projektu" style="width: 20; height: 10;"/>
<img src="bell.json"   alt="Wektorowa animacja dzwonka" style="width: 12; height: 6;"/>
<video src="clip.mp4"  alt="Prezentacja wideo" style="width: 40; height: 20;"/>
```

* **Automatyczna detekcja protokołu:** OxiTerm dynamicznie dopasowuje format graficzny do możliwości Twojego terminala (Kitty Graphics Protocol → Sixel → znaki Unicode `▀▄█`).
* **Formaty:** Obsługiwane są obrazy SVG, PNG, JPG oraz animacje Lottie (`.json`).
* **Wideo:** Odtwarzanie wideo wymaga obecności narzędzia `ffmpeg` w zmiennej środowiskowej `PATH`.

---

## 9. Klawiszologia

| Klawisz | Działanie |
|---------|-----------|
| `Tab` / `↓` / `→` | Przejście do następnego interaktywnego elementu (focus) |
| `Shift + Tab` / `↑` / `←` | Powrót do poprzedniego elementu |
| `Enter` | Aktywacja zaznaczonego elementu |
| `PgUp` / `b` | Przewinięcie widoku o jedną stronę w górę |
| `PgDn` / `Spacja` | Przewinięcie widoku o jedną stronę w dół |
| `Q` / `q` | Zamknięcie sesji i odłączenie |

---

## 10. Nawigacja między stronami

Możesz zmieniać całe ekrany, wskazując plik `.thtml` w akcji `event-htmx`:

```html
<text event-htmx="settings.thtml" style="height: 1;">→ Przejdź do ustawień</text>
```

> [!TIP]
> Stan sesji (`StateManager`) jest współdzielony i **zachowywany** podczas nawigacji między stronami pliku `.thtml`. Zmienne ustawione na jednej stronie będą dostępne na nowo załadowanej stronie.

---

## 11. Integracja z zewnętrznym serwerem aplikacji (App Server)

Jeśli Twoja aplikacja wymaga złożonej logiki biznesowej, dostępu do bazy danych lub zewnętrznych API, możesz podłączyć zewnętrzny serwer aplikacji. 

Uruchom OxiTerm ze zmienną `OXITERM_APP_SERVER`:
```bash
OXITERM_APP_SERVER=http://localhost:3000/events oxiterm serve app.thtml
```

Przy każdej akcji użytkownika (`event-htmx`), OxiTerm wyśle asynchroniczny (fire-and-forget) POST JSON do zdefiniowanego adresu url:
```json
{
  "action": "save_profile",
  "state": { "username": "admin", "email": "user@example.com" },
  "session_id": 42
}
```
Więcej informacji znajdziesz w dokumencie [app-server-guide.md](app-server-guide.md).

---

## 12. Zmienne Środowiskowe

Możesz konfigurować zachowanie serwera za pomocą poniższych zmiennych:

| Zmienna | Domyślnie | Opis |
|---------|-----------|------|
| `OXITERM_PORT` | `2222` | Port, na którym nasłuchuje serwer SSH |
| `OXITERM_HOST` | `0.0.0.0` | Adres IP nasłuchu SSH |
| `OXITERM_PASSWORD` | (brak) | Hasło do logowania SSH (jeśli ustawione, włącza autoryzację hasłem) |
| `OXITERM_NO_AUTH` | `false` | Wyłącza autoryzację SSH (wymagane w trybie deweloperskim, gdy brak kluczy autoryzowanych) |
| `OXITERM_WEB_PORT` | `8080` | Port serwera HTTP/WebSocket dla dostępu przez przeglądarkę |
| `OXITERM_APP_SERVER` | (brak) | Adres URL zewnętrznego serwera aplikacji dla akcji event-htmx |
| `RUST_LOG` | `warn` | Poziom logowania serwera (np. `debug`, `info`, `warn`, `error`) |
