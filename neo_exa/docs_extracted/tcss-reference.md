# TCSS — Terminal CSS Styling Reference

> Dokumentacja opisuje stan implementacji OxiTerm v0.3.x (aktualna).

---

## Czym jest TCSS?

TCSS to podzbiór CSS dostosowany do środowiska terminala. Style definiujesz na dwa sposoby:

**Inline** (najwyższy priorytet):
```html
<box style="fg: #38bdf8; bg: #0f172a; padding: 2;">
```

**Blok `<style>`** z selektorami (tag, klasa, id):
```html
<style>
.card { border-style: rounded; padding: 1; }
#header { fg: #f8fafc; }
text { fg: #e2e8f0; }
</style>
```

Kaskada: tag < klasa < id < inline.

**Kluczowa różnica od CSS:** jednostką jest **znak** (komórka terminala), nie piksel. `width: 40` = 40 kolumn terminala, `height: 3` = 3 wiersze.

---

## Właściwości — Wymiary

### `width: N`
Szerokość w kolumnach. Bez `width` węzeł rozciąga się do dostępnego miejsca.

### `height: N`
Wysokość w wierszach.

> **Zasada:** Zawsze ustawiaj `height: 1` na `<text>`. Bez tego Taffy może obliczyć wysokość 0 i tekst nie będzie widoczny.

---

## Właściwości — Kolory

### `fg` / `color`
Kolor pierwszego planu (tekst, znaki ramki). Dziedziczy się przez całe poddrzewo.

### `bg` / `background-color`
Kolor tła. Dziedziczy się przez całe poddrzewo.

Kolor `reset` lub `transparent` oznacza "użyj koloru rodzica".

### Formaty kolorów

**Hex RGB** (zalecany):
```
#rrggbb  →  #ff0000, #1e293b, #38bdf8
```

**Kolory nazwane:**
```
black  red  green  yellow  blue  magenta  cyan  white
```

**256-color palette** (liczba 0–255):
```
16–231  → sześcian RGB 6×6×6
232–255 → skala szarości
```

Przydatne wartości:
```
234–238  tła ciemne        240–245  szarości średnie
248–253  szarości jasne    226      żółty
46       zieleń jaskrawa   196      czerwień jaskrawa
```

---

## Właściwości — Layout (Flexbox)

OxiTerm używa Flexbox przez Taffy — semantycznie identyczny z CSS Flexbox.

### `flex-direction`

| Wartość | Opis |
|---------|------|
| `row` | Poziomo (domyślne) |
| `column` | Pionowo |

### `justify-content`

Rozmieszczenie wzdłuż głównej osi.

| Wartość | Opis |
|---------|------|
| `flex-start` | Od początku (domyślne) |
| `flex-end` | Od końca |
| `center` | Wyśrodkowanie |
| `space-between` | Równomierne, bez marginesów |
| `space-around` | Równomierne z marginesami |

### `align-items`

Wyrównanie w poprzek osi.

| Wartość | Opis |
|---------|------|
| `flex-start` | Do początku |
| `flex-end` | Do końca |
| `center` | Wyśrodkowanie |
| `stretch` | Rozciągnięcie (domyślne) |

---

## Właściwości — Odstępy

Wszystkie wartości to liczby całkowite (znaki / wiersze).

| Właściwość | Opis |
|------------|------|
| `padding: N` | Padding ze wszystkich stron |
| `padding-top: N` | ✅ działa |
| `padding-right: N` | ✅ działa |
| `padding-bottom: N` | ✅ działa |
| `padding-left: N` | ✅ działa |
| `margin: N` | Margin ze wszystkich stron |
| `margin-top: N` | ✅ działa |
| `margin-right: N` | ✅ działa |
| `margin-bottom: N` | ✅ działa |
| `margin-left: N` | ✅ działa |

> **Uwaga:** Węzeł z `border-style` automatycznie odsuwa zawartość o 1 znak ze wszystkich stron. Padding dodaje się do tego odsunięcia.

---

## Właściwości — Ramki

### `border-style`

| Wartość | Znaki | Wygląd |
|---------|-------|--------|
| `single` | `┌─┐│└─┘│` | Cienka ramka |
| `rounded` | `╭─╮│╰─╯│` | Zaokrąglone rogi |
| `double` | `╔═╗║╚═╝║` | Gruba/podwójna |

### `border-color`
Kolor ramki. Wymaga wcześniejszego `border-style`.

### `border`
Skrót — tworzy `single` ramkę z podanym kolorem.

Przykład:
```html
<box style="border-style: rounded; border-color: #475569; padding: 1;">
    <text style="height: 1;">Zawartość</text>
</box>
```

---

## Pełna tabela właściwości

| Właściwość | Wartości | Status |
|------------|----------|--------|
| `width` | int | ✅ |
| `height` | int | ✅ |
| `fg` / `color` | kolor | ✅ dziedziczony przez poddrzewo |
| `bg` / `background-color` | kolor | ✅ dziedziczony przez poddrzewo |
| `flex-direction` | `row`, `column` | ✅ |
| `justify-content` | 5 wartości | ✅ |
| `align-items` | 4 wartości | ✅ |
| `padding` | int | ✅ |
| `padding-top/right/bottom/left` | int | ✅ |
| `margin` | int | ✅ |
| `margin-top/right/bottom/left` | int | ✅ |
| `border-style` | `single`, `rounded`, `double` | ✅ renderowane |
| `border-color` | kolor | ✅ |
| `border` | kolor | ✅ skrót |

---

## Co NIE działa

Poniższe właściwości są nierealizowalne lub bez sensu w terminalu:

| Właściwość | Powód |
|------------|-------|
| `position: absolute/relative` | Tylko Flexbox |
| `z-index` | Brak warstw |
| `display: none` | Niezaimplementowane |
| `font-size`, `font-weight`, `font-style` | Terminal decyduje o wyglądzie czcionki |
| `opacity`, `filter`, `box-shadow` | Brak grafiki bitmapowej |
| `grid-template-*` | Tylko Flexbox |
| `overflow: scroll` | Brak mechanizmu przewijania |
| `border-radius` | Ramki to znaki Unicode |
| `transition`, `animation` | Animacje CSS (jest Lottie dla `<img>`) |
| Zewnętrzny plik `.tcss` | Tylko inline i `<style>` blok wewnątrz `.thtml` |

Selektory CSS obsługiwane w `<style>` bloku:

| Selektor | Status |
|----------|--------|
| Tag (`text { }`) | ✅ |
| Klasa (`.btn { }`) | ✅ |
| Id (`#main { }`) | ✅ |
| Descendant (`box text { }`) | ❌ nie obsługiwane |
| Pseudo-klasy (`:hover`, `:focus`) | ❌ nie obsługiwane |
| `@media` queries | ❌ nie obsługiwane |

---

## Paleta kolorów — ściągawka

```
Tła ciemne:
#0f172a  slate-900     #1e293b  slate-800     #334155  slate-700

Tekst:
#f8fafc  slate-50      #e2e8f0  slate-200     #94a3b8  slate-400
#64748b  slate-500

Akcenty:
#38bdf8  sky-400       #22d3ee  cyan-400      #4ade80  green-400
#f59e0b  amber-400     #f87171  red-400       #c084fc  purple-400
#fb923c  orange-400    #a3e635  lime-400
```
