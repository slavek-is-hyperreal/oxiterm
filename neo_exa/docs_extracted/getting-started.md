# Przewodnik: Tworzenie aplikacji z OxiTerm

Piszesz interfejs jak stronę WWW — plik `.thtml`, serwer go renderuje, łączysz się przez SSH.

---

## Szybki start

```bash
oxiterm serve ./app.thtml --port 2222
ssh localhost -p 2222
```

Edytuj plik, zapisz — terminal odświeża się automatycznie (**Hot Reload**). Stan sesji jest przy tym zachowany.

Inne tryby uruchamiania:
```bash
oxiterm serve app.thtml --no-auth          # bez hasła (dev)
oxiterm serve app.thtml --a11y             # tryb dostępności (tekst liniowy)
oxiterm serve app.thtml --web-port 8080    # jednocześnie przez SSH i przeglądarkę
oxiterm demo                               # wbudowana aplikacja pogodowa
oxiterm check app.thtml                    # walidacja składni bez uruchamiania
```

---

## Mentalny model

**Terminal to siatka znaków.** `width: 40` = 40 kolumn, `height: 3` = 3 wiersze.

Layout działa przez **Flexbox** — identyczny z CSS Flexbox. Jeśli znasz CSS Flexbox, znasz układ OxiTerm.

**Trzy rzeczy zawsze pamiętaj:**
1. `height: 1` na każdym `<text>`
2. `flex-direction: column` gdy chcesz elementy jeden pod drugim
3. `<style>` blok gdy chcesz reużywać styli przez klasy

---

## Tutorial 1 — Hello World

```html
<box style="
    width: 40; height: 10;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    bg: #1e293b;
    border-style: rounded;
    border-color: #475569;
">
    <text style="fg: #f8fafc; height: 1;">👋 Witaj w OxiTerm!</text>
    <text style="fg: #94a3b8; height: 1; margin-top: 1;">Naciśnij Q żeby wyjść</text>
</box>
```

---

## Tutorial 2 — Layout z klasami CSS

```html
<style>
.screen {
    flex-direction: column;
    bg: #0f172a;
}
.header {
    height: 3;
    bg: #1e3a5f;
    align-items: center;
    justify-content: space-between;
    padding-left: 2;
    padding-right: 2;
}
.content {
    flex-direction: column;
    padding: 2;
}
.footer {
    height: 1;
    bg: #1e293b;
    align-items: center;
    padding-left: 2;
}
</style>

<box class="screen" style="width: 80; height: 24;">
    <box class="header">
        <text style="fg: #38bdf8; height: 1;">🚀 Moja Aplikacja</text>
        <text style="fg: #64748b; height: 1;">v1.0</text>
    </box>

    <box class="content">
        <text style="fg: #e2e8f0; height: 1;">Tu jest treść.</text>
    </box>

    <box class="footer">
        <text style="fg: #475569; height: 1;">[Q] Wyjście  [Tab] Nawigacja</text>
    </box>
</box>
```

---

## Tutorial 3 — Interaktywny licznik

```html
<box style="
    flex-direction: column; width: 50; height: 16;
    bg: #0f172a; border-style: rounded; border-color: #334155; padding: 2;
">
    <text style="fg: #94a3b8; height: 1; margin-bottom: 1;">Licznik kliknięć:</text>

    <box style="justify-content: center; height: 3; align-items: center;">
        <text bind-state="clicks" style="fg: #22d3ee; height: 1;">0</text>
    </box>

    <box style="flex-direction: row; justify-content: center; height: 3; margin-top: 1;">
        <button event-htmx="dec:clicks"
                style="border-style: single; border-color: #f87171;
                       padding-left: 2; padding-right: 2; fg: #f87171;">
            −
        </button>
        <box style="width: 3;"></box>
        <button event-htmx="inc:clicks"
                style="border-style: single; border-color: #4ade80;
                       padding-left: 2; padding-right: 2; fg: #4ade80;">
            +
        </button>
        <box style="width: 3;"></box>
        <button event-htmx="clear:clicks"
                style="border-style: single; border-color: #475569;
                       padding-left: 2; padding-right: 2; fg: #475569;">
            ×
        </button>
    </box>
</box>
```

Kliknięcia myszą i klawiatura (`Tab` → focus, `Enter` → akcja) — oba działają.

---

## Tutorial 4 — Wszystkie typy stanu

```html
<box style="flex-direction: column; width: 60; height: 22; bg: #0f172a; padding: 2;">

    <!-- Int -->
    <box style="flex-direction: row; height: 1; align-items: center; margin-bottom: 1;">
        <text style="fg: #94a3b8; width: 20; height: 1;">Liczba:</text>
        <text bind-state="liczba" style="fg: #38bdf8; height: 1;">0</text>
        <box style="width: 2;"></box>
        <text event-htmx="inc:liczba" style="fg: #4ade80; height: 1;">[+]</text>
        <box style="width: 1;"></box>
        <text event-htmx="dec:liczba" style="fg: #f87171; height: 1;">[-]</text>
    </box>

    <!-- Bool -->
    <box style="flex-direction: row; height: 1; align-items: center; margin-bottom: 1;">
        <text style="fg: #94a3b8; width: 20; height: 1;">Włączony:</text>
        <text bind-state="wlaczony" style="fg: #fbbf24; height: 1;">false</text>
        <box style="width: 2;"></box>
        <text event-htmx="toggle:wlaczony" style="fg: #94a3b8; height: 1;">[przełącz]</text>
    </box>

    <!-- Str -->
    <box style="flex-direction: row; height: 1; align-items: center; margin-bottom: 1;">
        <text style="fg: #94a3b8; width: 20; height: 1;">Status:</text>
        <text bind-state="status" style="fg: #c084fc; height: 1;">nieznany</text>
        <box style="width: 2;"></box>
        <text event-htmx="set:status=ok" style="fg: #4ade80; height: 1;">[ok]</text>
        <box style="width: 1;"></box>
        <text event-htmx="set:status=błąd" style="fg: #f87171; height: 1;">[błąd]</text>
    </box>

    <!-- List -->
    <box style="flex-direction: row; height: 1; align-items: center; margin-bottom: 1;">
        <text style="fg: #94a3b8; width: 20; height: 1;">Lista:</text>
        <text bind-state="lista" style="fg: #34d399; height: 1;">[]</text>
    </box>
    <box style="flex-direction: row; height: 1; margin-left: 20;">
        <text event-htmx="append:lista=elem" style="fg: #94a3b8; height: 1;">[dodaj]</text>
        <box style="width: 2;"></box>
        <text event-htmx="clear:lista" style="fg: #94a3b8; height: 1;">[wyczyść]</text>
    </box>

</box>
```

---

## Tutorial 5 — Nawigacja wielostronicowa

```html
<!-- index.thtml -->
<box style="flex-direction: column; width: 40; height: 20;
            bg: #0f172a; border-style: rounded; border-color: #334155; padding: 2;">

    <text style="fg: #f8fafc; height: 1; margin-bottom: 2;">Menu główne</text>

    <box event-htmx="licznik.thtml"
         style="border-style: single; border-color: #334155; padding-left: 2;
                height: 3; align-items: center; margin-bottom: 1;">
        <text style="fg: #38bdf8; height: 1;">🔢 Licznik</text>
    </box>

    <box event-htmx="ustawienia.thtml"
         style="border-style: single; border-color: #334155; padding-left: 2;
                height: 3; align-items: center;">
        <text style="fg: #f59e0b; height: 1;">⚙️  Ustawienia</text>
    </box>

</box>
```

Stan sesji jest **zachowany** między stronami — wartości ustawione na jednej stronie są dostępne na kolejnej.

---

## Wzorce projektowe

### Panel z tytułem

```html
<box style="border-style: rounded; border-color: #334155;
            padding: 1; flex-direction: column;">
    <text style="fg: #64748b; height: 1; margin-bottom: 1;">─── Tytuł ───</text>
    <text style="fg: #e2e8f0; height: 1;">Zawartość</text>
</box>
```

### Pasek statusu

```html
<box style="width: 80; height: 1; bg: #1e293b; flex-direction: row;
            justify-content: space-between; padding-left: 1; padding-right: 1;">
    <text style="fg: #4ade80; height: 1;">● Połączony</text>
    <text bind-state="czas" style="fg: #475569; height: 1;">--:--</text>
</box>
```

### Tabela

```html
<box style="flex-direction: column; padding: 1;">
    <!-- Nagłówek -->
    <box style="flex-direction: row; height: 1; bg: #1e293b;">
        <text style="width: 20; height: 1; fg: #94a3b8;">Imię</text>
        <text style="width: 15; height: 1; fg: #94a3b8;">Wynik</text>
    </box>
    <text style="fg: #334155; height: 1;">────────────────────────────────</text>
    <!-- Wiersze -->
    <box style="flex-direction: row; height: 1;">
        <text style="width: 20; height: 1; fg: #e2e8f0;">Anna K.</text>
        <text style="width: 15; height: 1; fg: #4ade80;">9850</text>
    </box>
</box>
```

### Spacer / separator

```html
<box style="width: 2;"></box>    <!-- poziomy spacer -->
<box style="height: 1;"></box>   <!-- pionowy spacer -->
```

---

## Najczęstsze błędy

| Problem | Przyczyna | Fix |
|---------|-----------|-----|
| Tekst niewidoczny | Brak `height: 1` na `<text>` | Dodaj `height: 1` |
| Elementy obok siebie zamiast pod sobą | Domyślny `flex-direction: row` | Dodaj `flex-direction: column` |
| Klasy bez efektu | Brak bloku `<style>` w pliku | Dodaj `<style>.klasa { }</style>` |
| Kliknięcia nie reagują | Terminal bez obsługi myszy | Użyj Kitty/WezTerm/Ghostty/Alacritty |
| Ramka "zjada" treść | `border` + `padding: 1` = 2 znaki odstępu | Usuń padding lub zwiększ wymiary |

---

## Znane ograniczenia

- **Brak przewijania** — `overflow: scroll` nie istnieje. Treść dłuższa niż ekran jest obcinana.
- **Brak zawijania tekstu** — tekst dłuższy niż szerokość węzła jest obcinany. Podziel ręcznie na wiele `<text>`.
- **Obrazy Kitty nie znikają** — po nawigacji do innej strony obrazy mogą pozostać jako "duchy" (naprawa planowana).
- **`<video>` wymaga ffmpeg** — bez zainstalowanego `ffmpeg` w systemie wideo nie działa.
- **Obrazy niewidoczne w przeglądarce** — tryb `--web-port` wyświetla tylko tekst; grafika wymaga dodatkowego mechanizmu.
- **Brak `class` bez `<style>` bloku** — atrybut `class` bez odpowiadającego bloku `<style>` jest ignorowany.

---

## Hot Reload

Przy każdym zapisie pliku wszyscy połączeni klienci widzą aktualizację natychmiast. Stan sesji (liczniki, flagi, listy) jest **zachowywany** przy przeładowaniu — możesz modyfikować układ bez resetowania stanu aplikacji.
