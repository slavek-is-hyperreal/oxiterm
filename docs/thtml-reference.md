# THTML — Terminal HTML Language Reference

THTML (Terminal HTML) to deklaratywny język znaczników OxiTerm służący do budowania interfejsów TUI (Terminal User Interface) renderowanych po stronie serwera. Jest to podzbiór XML ze ścisłą strukturą tagów oraz małym, wyspecjalizowanym słownikiem atrybutów.

---

## 1. Struktura dokumentu

Każdy dokument THTML jest parsuwany do drzewa węzłów, którego domyślnym, ukrytym korzeniem jest węzeł `<screen>`. W dowolnym miejscu dokumentu (zazwyczaj na początku) może pojawić się opcjonalny blok `<style>` zawierający definicje reguł stylizowania TCSS.

```html
<!-- Opcjonalny blok stylów -->
<style>
  .btn { border-style: single; border-color: #4ade80; }
  .header-text { fg: #38bdf8; height: 1; }
</style>

<!-- Drzewo komponentów interfejsu -->
<box style="flex-direction: column; width: 80; height: 24; bg: #0f172a;">
  <text class="header-text">Witaj w OxiTerm!</text>
</box>
```

Pliki nie powinny zawierać elementów specyficznych dla przeglądarkowego HTML, takich jak `<!DOCTYPE>`, `<html>`, `<head>` czy `<body>`.

---

## 2. Obsługiwane Tagi

OxiTerm wspiera dokładnie siedem tagów. Użycie nieznanego tagu spowoduje błąd parsowania dokumentu.

| Tag | Rola | Dzieci | Uwagi |
|---|---|---|---|
| `<screen>` | Niejawny korzeń | Dowolne | Tworzony automatycznie przez parser, nigdy nie pisze się go wprost. |
| `<box>` | Kontener układu | Dowolne | Odpowiednik HTML-owego `<div>`. Domyślnie działa jako kontener Flexbox. |
| `<text>` | Tekst wierszowy | Brak (tylko tekst) | Wyświetla treść tekstową. Wspiera Unicode, znaki dwukomórkowe (CJK) oraz emoji. |
| `<input>` | Pole tekstowe | Brak | Służy do pobierania tekstu od użytkownika. Wymaga atrybutu `bind-value`. |
| `<button>` | Interaktywny przycisk | Brak (tylko tekst) | Węzeł focusowalny klawiaturą, uruchamia akcję zdefiniowaną w `event-htmx`. |
| `<img>` | Obraz lub animacja | Brak | Wyświetla pliki SVG, PNG, JPG oraz animacje Lottie (`.json`). |
| `<video>` | Odtwarzacz wideo | Brak | Płynne odtwarzanie filmów w tle. Wymaga obecności programu `ffmpeg` w systemie. |

> [!WARNING]
> Tagi `<img>` oraz `<input>` mogą być samozamykające się (`<img />`, `<input />`). Pozostałe tagi bezwzględnie wymagają tagów zamykających (np. `<text>Zawartość</text>`).

---

## 3. Atrybuty Uniwersalne

Poniższe atrybuty mogą być aplikowane do każdego typu węzła:

| Atrybut | Typ wartości | Opis |
|---|---|---|
| `id` | Tekst | Unikalny identyfikator węzła w drzewie. Wykorzystywany przez selektor CSS `#id`. |
| `class` | Tekst (rozdzielany spacjami) | Klasy stylów TCSS. Aplikowane zgodnie z kolejnością występowania w arkuszu stylów. |
| `style` | Styl TCSS | Deklaracje stylów inline (rozdzielane średnikami). Mają najwyższy priorytet w kaskadzie. |
| `event-htmx` | Tekst akcji | Akcja (lub lista akcji) wywoływana po kliknięciu elementu lub wciśnięciu klawisza `Enter`. |
| `bind-state` | Klucz stanu | Reaktywnie wyświetla bieżącą wartość powiązanego klucza ze `StateManager` jako treść tekstową. |
| `bind-show` | Tekst warunku | Warunkowo ukrywa/pokazuje węzeł. Węzeł ukryty jest całkowicie usuwany z layoutu. |

---

## 4. Atrybuty Specyficzne dla Tagów

| Tag | Atrybut | Opis |
|---|---|---|
| `<img>`, `<video>` | `src` | Ścieżka relatywna do pliku zasobu (względem katalogu z plikiem `.thtml`). Ścieżki są sprawdzane pod kątem prób ataków typu Path Traversal. |
| `<img>`, `<video>` | `alt` | Tekstowy opis elementu multimedialnego, wykorzystywany w trybie ułatwień dostępu (`--a11y`). |
| `<input>` | `placeholder` | Tekst pomocniczy wyświetlany w polu, gdy bufor wpisywania jest pusty. |
| `<input>` | `name` | Nazwa maszynowa pola wejściowego używana jako etykieta w drzewie ułatwień dostępu (Accessibility Tree). |
| `<input>` | `bind-value` | Klucz stanu w `StateManager`, w którym **na bieżąco** (przy każdym wciśnięciu klawisza) zapisywany jest wpisywany tekst. |

---

## 5. Akcje `event-htmx`

Atrybut `event-htmx` obsługuje pojedynczą akcję lub ich sekwencję. Instrukcje wykonywane są od lewej do prawej. Jako separatora instrukcji można używać zarówno **średnika** (`;`), jak i **przecinka** (`,`).

| Format akcji | Efekt działania | Przykład użycia |
|---|---|---|
| `inc:klucz` | Zwiększa wartość całkowitą stanu (Int) o 1 | `event-htmx="inc:counter"` |
| `dec:klucz` | Zmniejsza wartość całkowitą stanu (Int) o 1 | `event-htmx="dec:counter"` |
| `toggle:klucz` | Przełącza wartość logiczną stanu (Bool) | `event-htmx="toggle:sidebar_open"` |
| `set:klucz=wartość` | Ustawia stan (Str) na podany tekst | `event-htmx="set:tab=settings"` |
| `append:klucz=wartość` | Dodaje podaną wartość tekstową do listy (List) | `event-htmx="append:logs=nowe_zdarzenie"` |
| `clear:klucz` | Resetuje stan do wartości domyślnej dla danego typu (`0`, `false`, `""` lub `[]`) | `event-htmx="clear:counter"` |
| `plik.thtml` | Zmienia aktualny ekran aplikacji na inny plik `.thtml` | `event-htmx="dashboard.thtml"` |
| `akcja1;akcja2` | Wykonuje wiele akcji kolejno po sobie | `event-htmx="set:tab=x;inc:views"` |

> [!NOTE]
> Przejście do innej strony `.thtml` (nawigacja) **zachowuje** cały stan zgromadzony w pamięci podręcznej sesji `StateManager`. Dzięki temu użytkownik nie traci wprowadzonych danych ani kontekstu sesji podczas przechodzenia między ekranami.

---

## 6. Warunki `bind-show`

Atrybut `bind-show` służy do reaktywnego ukrywania elementów interfejsu. Węzły, których warunek ewaluuje się do `false`, są całkowicie usuwane z kalkulacji layoutu Taffy (nie zajmują żadnej przestrzeni w terminalu).

| Składnia warunku | Warunek jest prawdziwy (`true`), gdy: |
|---|---|
| `bind-show="klucz"` | Powiązany klucz ma wartość logiczną `true`, niezerową liczbę całkowitą (Int != 0), niepusty ciąg znaków (Str != "" i Str != "false") lub niepustą listę. |
| `bind-show="klucz=wartość"` | Reprezentacja tekstowa wartości stanu pod tym kluczem jest dokładnie równa `"wartość"`. Jeśli wartością stanu pod kluczem jest lista (`List`), warunek jest spełniony, gdy lista zawiera element o wartości `"wartość"`. |
| `bind-show="klucz=false"` | Wartość powiązanego klucza jest fałszywa (falsy) lub **klucz w ogóle nie istnieje w stanie**. |
| `bind-show="klucz=true"` | Wartość powiązanego klucza jest prawdziwa (truthy). Jeśli klucz nie istnieje, zwracane jest `false`. |

> [!IMPORTANT]
> Jeżeli klucz stanu nie istnieje w pamięci podręcznej sesji, wszystkie formy sprawdzania warunku (oprócz `key=false`) domyślnie ewaluują się jako `false`. Użycie `bind-show="menu=false"` pozwala na domyślne wyświetlenie panelu przy starcie sesji, zanim stan zostanie zainicjalizowany przez jakiekolwiek akcje.

---

## 7. Typy Stanu

`StateManager` przechowuje typowane wartości stanu. W poniższej tabeli przedstawiono, jak poszczególne typy są renderowane na ekranie przez atrybut `bind-state`:

| Typ stanu | Wewnętrzny typ Rust | Sposób reprezentacji przez `bind-state` | Przykład wyjścia |
|---|---|---|---|
| **Int** | `i64` | Jako zapis liczbowy | `42` |
| **Bool** | `bool` | W postaci słownej: `true` lub `false` | `true` |
| **Str** | `String` | Bezpośrednia treść napisu | `Witaj świecie` |
| **List** | `Vec<String>` | Lista wartości ujęta w nawiasy kwadratowe | `[element1, element2, element3]` |

---

## 8. Interakcja z Polami `<input>`

Przepływ wprowadzania danych w polach tekstowych wygląda następująco:
1. Użytkownik przenosi zaznaczenie (focus) na pole `<input>` za pomocą klawisza `Tab` lub strzałek kierunkowych.
2. Zaczyna wpisywać tekst. Znak po znaku, wprowadzana treść jest **natychmiast zapisywana** w stanie pod kluczem zdefiniowanym w atrybucie `bind-value`. Inne elementy powiązane z tym samym kluczem poprzez `bind-state` będą na żywo reagować na każdy wpisany znak.
3. Klawisz `Backspace` usuwa ostatni znak z bufora i aktualizuje stan.
4. Klawisz `Enter` zatwierdza pole i uruchamia akcję zdefiniowaną w atrybucie `event-htmx` (jeśli taki istnieje).

Przykład powiązania wejścia z reaktywnym tekstem:
```html
<input bind-value="email" placeholder="Wpisz swój email..."
       style="height: 1; border-style: single; border-color: #38bdf8;"/>
<text bind-state="email" style="fg: #94a3b8; height: 1;"/>
```

---

## 9. Komentarze

Standardowe komentarze znane z języka HTML są całkowicie pomijane podczas parsowania drzewa dokumentu:
```html
<!-- Ten komentarz zostanie wycięty i nie trafi do drzewa DOM -->
```

---

## 10. Bezpieczeństwo i sanityzacja danych

OxiTerm dba o to, by wejście dostarczane przez pliki THTML i interakcję użytkownika nie zepsuło wyświetlania u innych klientów ani nie doprowadziło do błędów renderowania:
* **Style (`style="..."`):** Wszelkie kody ucieczki ANSI (Escape sequences) są wykrywane i wycinane za pomocą wyrażenia regularnego. Unika to potencjalnych ataków polegających na wstrzyknięciu destrukcyjnych kodów sterujących terminalem.
* **Atrybuty akcji (`event-htmx`, `bind-state`, `bind-show`):** Wszystkie znaki kontrolne oraz znaki takie jak nawiasy, cudzysłowy i ukośniki wsteczne (`(`, `)`, `'`, `"`, `<`, `>`, `\`, `` ` ``) są usuwane. Dozwolone są wyłącznie znaki alfanumeryczne, spacje, znaki równości oraz znaki bezpieczne dla adresów URL.
