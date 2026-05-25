# TCSS — Terminal CSS Styling Reference

TCSS (Terminal CSS) to język stylizowania używany w OxiTerm do kontrolowania układu (layoutu), odstępów oraz kolorów elementów THTML. Składnią bezpośrednio nawiązuje do standardowego języka CSS, jednak jest dostosowany do siatki znakowej terminala i ograniczeń graficznych środowiska tekstowego.

---

## 1. Składnia i Kaskada

Stylizację można definiować na dwa sposoby:
1. W bloku `<style>` umieszczonym wewnątrz dokumentu THTML.
2. Bezpośrednio na węzłach za pomocą atrybutu `style` (style inline).

### Selektory i Ważność (Priorytet kaskady)

Style są aplikowane w jednej zunifikowanej fazie według następującego rosnącego priorytetu (od najsłabszego do najsilniejszego):
1. **Selektor Tagu:** np. `text { fg: green; }` (aplikuje się do wszystkich tagów tego typu).
2. **Selektor Klasy:** np. `.btn { bg: blue; }` (aplikuje się do węzłów posiadających atrybut `class="btn"`).
3. **Selektor Identyfikatora:** np. `#main { width: 80; }` (aplikuje się do węzła z `id="main"`).
4. **Style inline:** np. `<box style="height: 3;">` (zdefiniowane bezpośrednio w atrybucie `style` węzła).

Kolejne reguły o tym samym priorytecie nadpisują poprzednie (decyduje kolejność zapisu w pliku).

---

## 2. Obsługiwane Właściwości

| Właściwość | Dozwolone Wartości | Opis |
|---|---|---|
| `width` | Liczba całkowita | Szerokość elementu wyrażona w liczbie kolumn terminala. |
| `height` | Liczba całkowita | Wysokość elementu wyrażona w liczbie wierszy terminala. |
| `fg` \| `color` | Kolor | Kolor tekstu (foreground). |
| `bg` \| `background-color` | Kolor | Kolor tła (background). |
| `flex-direction` | `row` (domyślnie), `column` | Kierunek układania elementów potomnych w kontenerze Flexbox. |
| `align-items` | `flex-start` (domyślnie), `flex-end`, `center`, `stretch` | Wyrównanie elementów potomnych w poprzek osi głównej. |
| `justify-content` | `flex-start` (domyślnie), `flex-end`, `center`, `space-between`, `space-around` | Rozmieszczenie elementów potomnych wzdłuż osi głównej. |
| `padding` | Liczba całkowita | Wewnętrzny odstęp (ze wszystkich stron) wyrażony w komórkach znakowych. |
| `padding-top` \| `padding-right` \| `padding-bottom` \| `padding-left` | Liczba całkowita | Szczegółowe wewnętrzne odstępy z poszczególnych stron. |
| `margin` | Liczba całkowita | Zewnętrzny margines (ze wszystkich stron) wyrażony w komórkach znakowych. |
| `margin-top` \| `margin-right` \| `margin-bottom` \| `margin-left` | Liczba całkowita | Szczegółowe zewnętrzne marginesy z poszczególnych stron. |
| `border` | Kolor | Włącza ramkę wokół elementu o określonym kolorze i domyślnym stylu (`single`). |
| `border-style` | `single`, `double`, `rounded` | Styl znaków użytych do narysowania ramki (znaki rysowania ramek Unicode). |
| `border-color` | Kolor | Określa lub nadpisuje kolor ramki. |

---

## 3. Definiowanie Kolorów

W TCSS kolory można definiować na cztery sposoby:

| Format koloru | Opis | Przykład |
|---|---|---|
| **Nazwa CSS** | Jedna z podstawowych nazw kolorów (automatycznie mapowana na TrueColor RGB) | `fg: red;`, `bg: blue;` |
| **Hex RGB** | Zapis szesnastkowy o długości 7 znaków (24-bit TrueColor) | `fg: #ff5500;`, `bg: #0f172a;` |
| **Numer 0-255** | Indeks z 256-kolorowej palety ANSI (przydatne jako fallback na starszych terminalach) | `fg: 46;` (jasny zielony), `bg: 234;` (ciemnoszary) |
| **Reset / Transparent** | Przywraca domyślny kolor terminala lub czyni tło przeźroczystym | `fg: reset;`, `bg: transparent;` |

### Obsługiwane nazwy kolorów (CSS Color Names)
OxiTerm rozpoznaje dokładnie następujące nazwy kolorów:
* `black` (RGB 0, 0, 0)
* `red` (RGB 255, 0, 0)
* `green` (RGB 0, 255, 0)
* `yellow` (RGB 255, 255, 0)
* `blue` (RGB 0, 0, 255)
* `magenta` (RGB 255, 0, 255)
* `cyan` (RGB 0, 255, 255)
* `white` (RGB 255, 255, 255)

---

## 4. Rysowanie Ramek (Borders)

Włączenie ramki na elemencie odbywa się za pomocą właściwości `border` lub `border-color`. Ramka zajmuje **dokładnie 1 komórkę znakową** szerokości i wysokości na krawędziach elementu.

### Style ramek (`border-style`):
Do rysowania ramek OxiTerm używa znaków semigraﬁcznych Unicode (Box Drawing Characters):

* **`single`** (domyślny):
  ```text
  ┌──────┐
  │      │
  └──────┘
  ```
* **`double`**:
  ```text
  ╔══════╗
  ║      ║
  ╚══════╝
  ```
* **`rounded`**:
  ```text
  ╭──────╮
  │      │
  ╰──────╯
  ```

---

## 5. Czego TCSS NIE obsługuje (w porównaniu do przeglądarkowego CSS)

* **Brak jednostek:** Rozmiary, marginesy i paddingi podaje się jako czyste liczby całkowite (reprezentujące komórki znakowe terminala). Jednostki takie jak `px`, `em`, `rem`, `%` nie są obsługiwane.
* **Brak `display: none`:** Ukrywanie elementów realizowane jest wyłącznie na poziomie struktury DOM za pomocą atrybutu `bind-show`.
* **Brak stylizacji tekstu:** Właściwości takie jak `font-size`, `font-family`, `font-weight` (np. bold), czy `font-style` (np. italic) nie są wspierane w TCSS, ponieważ terminal wymusza użycie czcionki o stałej szerokości (monospace).
* **Brak zaokrągleń rogów tła:** Właściwość `border-radius` nie jest obsługiwana. Jedynym sposobem na uzyskanie zaokrąglonych rogów jest użycie `border-style: rounded;`.
