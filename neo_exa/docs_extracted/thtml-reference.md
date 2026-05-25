# THTML — Terminal HTML Language Reference

> Dokumentacja opisuje stan implementacji OxiTerm v0.3.x (aktualna).

---

## Czym jest THTML?

THTML to uproszczony język znaczników wzorowany na HTML, zaprojektowany do opisywania interfejsów użytkownika renderowanych w terminalu. Plik `.thtml` opisuje układ, styl i interakcje — serwer OxiTerm parsuje go, oblicza layout przez Flexbox (biblioteka Taffy) i renderuje do siatki znaków terminala.

Kluczowa różnica od HTML: **wszystko mierzone jest w znakach, nie pikselach**. `width: 40` = 40 kolumn terminala.

---

## Struktura pliku

```html
<!-- opcjonalny blok CSS -->
<style>
.panel {
    border-style: rounded;
    border-color: #334155;
    padding: 1;
}
</style>

<!-- drzewo węzłów -->
<box class="panel" style="width: 60; height: 20; flex-direction: column;">
    <text style="fg: #38bdf8; height: 1;">Witaj!</text>
    <button event-htmx="inc:licznik">Kliknij</button>
    <text bind-state="licznik">0</text>
</box>
```

`<!DOCTYPE>`, `<html>`, `<head>` nie są obsługiwane i nie powinny być używane. Komentarze HTML (`<!-- -->`) są parsowane i ignorowane.

---

## Blok `<style>` (CSS kaskadowy)

Plik `.thtml` może zawierać jeden lub więcej bloków `<style>` przed lub wewnątrz drzewa węzłów. Parser wyciąga CSS z bloków przed budowaniem DOM, a następnie aplikuje reguły w kolejności kaskadowej.

```html
<style>
/* Selektor tagu */
text {
    fg: #e2e8f0;
}

/* Selektor klasy */
.highlighted {
    fg: #f59e0b;
    border-style: single;
}

/* Selektor id */
#title {
    fg: #38bdf8;
}
</style>
```

**Kolejność kaskady (od najniższego do najwyższego priorytetu):**

| Poziom | Przykład | Nadpisuje |
|--------|----------|-----------|
| Tag | `text { fg: red; }` | nic |
| Klasa | `.btn { fg: blue; }` | tag |
| Id | `#main { fg: green; }` | klasa |
| Inline | `style="fg: yellow;"` | wszystko |

Każdy węzeł może mieć wiele klas: `class="panel highlighted"`.

---

## Tagi

### `<box>`

Kontener layoutu (odpowiednik `<div>` z `display: flex`). Może zawierać dowolne inne tagi.

```html
<box style="flex-direction: column; padding: 2; bg: #0f172a;">
    <text style="height: 1;">Dziecko 1</text>
    <box style="flex-direction: row;">
        <text style="height: 1;">Lewo</text>
        <text style="height: 1;">Prawo</text>
    </box>
</box>
```

Domyślny `flex-direction`: `row`.

---

### `<text>`

Wyświetla treść tekstową. Zawsze ustawiaj `height: 1` dla pojedynczej linii.

```html
<text style="fg: #22c55e; height: 1;">Zielony tekst</text>
```

Obsługuje emoji i znaki Unicode dwukomórkowe (CJK) z poprawnym przesunięciem kursora:
```html
<text style="height: 1;">🚀 Rakieta zajmuje 2 kolumny</text>
<text style="height: 1;">日本語 — znaki CJK też dwukomórkowe</text>
```

Tekst który nie mieści się w szerokości węzła jest **obcinany** (brak automatycznego zawijania).

---

### `<button>`

Interaktywny przycisk. Działa identycznie jak `<box>`, ale semantycznie oznacza element klikalny. Ma sens tylko z `event-htmx`.

```html
<button event-htmx="inc:score"
        style="border-style: rounded; padding-left: 2; padding-right: 2; fg: #f59e0b;">
    [ + ]
</button>
```

Klawiaturowy focus (`Tab`/strzałki) zatrzymuje się na węzłach z `event-htmx` — zarówno `<button>` jak i `<box>` z tym atrybutem.

---

### `<input>`

Pole tekstowe. Renderuje się jako wiersz znaków `_`. Służy jako bufor Predictive Echo — wpisywane znaki pojawiają się natychmiast (lokalnie) zanim dotrą do serwera.

```html
<input name="query" placeholder="Szukaj..." style="width: 30; height: 1; fg: #a3e635;" />
```

Atrybuty specjalne dla `<input>`:
- `name` — używany przez a11y tree jako etykieta
- `placeholder` — używany przez a11y tree jeśli brak `name`

`<input>` nie przekazuje wartości do StateManager w obecnej wersji.

---

### `<img>`

Wyświetla obraz. OxiTerm wybiera protokół automatycznie:

| Terminal | Protokół | Jakość |
|----------|----------|--------|
| Kitty, WezTerm, Ghostty | Kitty Graphics | pełna RGBA |
| Konsole, mlterm | Sixel | 256 kolorów |
| Pozostałe | Unicode block (`▀▄█`) | przybliżona |

```html
<!-- Obraz rastrowy -->
<img src="logo.png" style="width: 40; height: 20;" />

<!-- SVG (renderowany przez resvg) -->
<img src="icon.svg" style="width: 10; height: 5;" />

<!-- Animacja Lottie (.json) — odtwarza się automatycznie w pętli -->
<img src="spinner.json" style="width: 8; height: 4;" />
```

Atrybut `alt` jest używany przez a11y tree i tryb linearny:
```html
<img src="earth.png" alt="Zdjęcie Ziemi z kosmosu" style="width: 40; height: 20;" />
```

**Ważne:** Ścieżka `src` jest relatywna względem pliku `.thtml`. Wyjście poza katalog pliku jest blokowane (ochrona path traversal).

Wymiary w `style` to **znaki**, nie piksele. OxiTerm przelicza: `pixel_w = width * 10`, `pixel_h = height * 20`.

Jeśli plik nie istnieje lub rendering się nie uda — czerwona ramka `*` z nazwą pliku.

> **Znana limitacja:** Po nawigacji do innej strony obrazy Kitty mogą zostać na ekranie jako "duchy". Wymagane wysłanie `ESC_G a=d,d=A` — naprawka planowana.

---

### `<video>`

Odtwarza plik wideo przez ffmpeg w tle. Wymaga `ffmpeg` w systemie.

```html
<video src="demo.mp4" alt="Demonstracja funkcji" style="width: 60; height: 20;" />
```

Obsługiwane formaty: wszystkie obsługiwane przez zainstalowany ffmpeg. Odtwarzanie zapętlone. Jeśli ffmpeg nie jest dostępny — czerwona ramka z komunikatem.

---

## Atrybuty

### `id`

Identyfikator węzła. Używany przez selektory CSS w bloku `<style>` (`#id { }`).

```html
<box id="main-panel">...</box>
```

---

### `class`

Lista klas oddzielona spacjami. Używana przez selektory CSS w bloku `<style>` (`.class { }`).

```html
<box class="panel highlighted">...</box>
```

> Bez bloku `<style>` w pliku atrybut `class` jest parsowany ale nie stosowany.

---

### `style`

Inline style TCSS. Pełna specyfikacja w [tcss-reference.md](tcss-reference.md). Najwyższy priorytet kaskady.

```html
<box style="fg: #38bdf8; bg: #0f172a; padding: 2; border-style: rounded;">
```

---

### `src`

Ścieżka do zasobu dla `<img>` i `<video>`. Zawsze relatywna do pliku `.thtml`.

---

### `alt`

Tekst alternatywny dla `<img>` i `<video>`. Używany w trybie a11y (`--a11y`) i przez `build_a11y_tree`.

---

### `name`

Dla `<input>` — nazwa pola używana przez a11y tree.

---

### `placeholder`

Dla `<input>` — placeholder używany przez a11y tree jeśli brak `name`.

---

### `event-htmx`

Akcja wykonywana po kliknięciu myszą **lub** po wciśnięciu Enter gdy węzeł ma focus klawiatury.

| Akcja | Opis | Przykład |
|-------|------|---------|
| `inc:klucz` | Inkrementuje `Int` | `event-htmx="inc:score"` |
| `dec:klucz` | Dekrementuje `Int` | `event-htmx="dec:lives"` |
| `toggle:klucz` | Przełącza `Bool` | `event-htmx="toggle:menu"` |
| `set:klucz=wartość` | Ustawia `Str` | `event-htmx="set:status=ok"` |
| `append:klucz=element` | Dodaje do `List` | `event-htmx="append:items=nowe"` |
| `clear:klucz` | Zeruje wartość (no-op dla nieistniejących) | `event-htmx="clear:score"` |
| `plik.thtml` | Nawigacja do innej strony | `event-htmx="settings.thtml"` |

**Event bubbling:** jeśli kliknięty węzeł nie ma `event-htmx`, zdarzenie wędruje do przodków (przez `parent_map`, O(głębokość)).

```html
<box event-htmx="inc:counter" style="padding: 1; border-style: rounded;">
    <text style="height: 1;">Kliknij gdziekolwiek tutaj</text>
</box>
```

---

### `bind-state`

Wiąże treść tekstową węzła z kluczem w StateManager. Aktualizacja jest reaktywna (zmiana stanu → aktualizacja węzła bez pełnego przeładowania).

```html
<text bind-state="score" style="height: 1;">0</text>
```

| Typ stanu | Wyświetla |
|-----------|-----------|
| `Int(42)` | `42` |
| `Str("hello")` | `hello` |
| `Bool(true)` | `true` |
| `List(["a","b"])` | `[a, b]` |

Subskrypcje są inicjalizowane przy starcie sesji — `bind-state` działa od pierwszej klatki.

---

## Nawigacja klawiaturą

Węzły z `event-htmx` są automatycznie dodawane do listy focusable. Nawigacja:

| Klawisz | Akcja |
|---------|-------|
| `Tab` lub `↓`/`→` | Następny focusable węzeł |
| `↑`/`←` | Poprzedni focusable węzeł |
| `Enter` | Aktywuj akcję `event-htmx` focused węzła |
| `Q`/`q` | Wyjście z sesji |

Focus ring jest renderowany jako `▶` i `◀` po bokach focused węzła (kolor: jasny cyan).

---

## Nawigacja między stronami

Stan sesji (StateManager) jest **zachowywany** przy nawigacji między stronami.

```html
<!-- strona A: ustaw i przejdź -->
<button event-htmx="set:user=admin">Zaloguj</button>
<button event-htmx="panel.thtml">Przejdź do panelu</button>
```

```html
<!-- panel.thtml: odczytaj stan -->
<text bind-state="user" style="height: 1;">nieznany</text>
```

Nawigacja jest zabezpieczona przed path traversal — nie można wyjść poza katalog bazowy pliku `.thtml`.

---

## Tagi samozamykające

`<img>` i `<input>` mogą być samozamykające (`/>`). Pozostałe wymagają tagu zamykającego.

---

## Ograniczenia parsera

- Obsługiwane tagi: `box`, `text`, `input`, `button`, `img`, `video`
- Każdy atrybut musi mieć wartość (`attr="val"`)
- Brak zawijania tekstu — tekst przekraczający szerokość jest obcinany
- Brak `overflow: scroll` — długie treści nie przewijają się
- `class` bez bloku `<style>` w pliku — nie stosowane
