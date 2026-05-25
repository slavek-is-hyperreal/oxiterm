# TCSS — Terminal CSS Styling Reference

TCSS (Terminal CSS) is the styling language used in OxiTerm to control the layout, spacing, and colors of THTML elements. Its syntax directly mirrors standard CSS, but is adapted to the terminal character grid and the graphical constraints of text-based environments.

---

## 1. Syntax and Cascade

Styling can be defined in two ways:
1. In a `<style>` block placed inside the THTML document.
2. Directly on nodes using the `style` attribute (inline styles).

### Selectors and Specificity (Cascade Priority)

Styles are applied in a single unified phase according to the following ascending priority (from weakest to strongest):
1. **Tag Selector:** e.g., `text { fg: green; }` (applies to all tags of this type).
2. **Class Selector:** e.g., `.btn { bg: blue; }` (applies to nodes with a `class="btn"` attribute).
3. **ID Selector:** e.g., `#main { width: 80; }` (applies to the node with `id="main"`).
4. **Inline Styles:** e.g., `<box style="height: 3;">` (defined directly in the node's `style` attribute).

Subsequent rules with the same priority overwrite previous ones (determined by the order of appearance in the file).

---

## 2. Supported Properties

| Property | Allowed Values | Description |
|---|---|---|
| `width` | Integer | Element width expressed as the number of terminal columns. |
| `height` | Integer | Element height expressed as the number of terminal rows. |
| `fg` \| `color` | Color | Text color (foreground). |
| `bg` \| `background-color` | Color | Background color. |
| `flex-direction` | `row` (default), `column` | The direction in which child elements are laid out in a Flexbox container. |
| `align-items` | `flex-start` (default), `flex-end`, `center`, `stretch` | Alignment of child elements cross the main axis. |
| `justify-content` | `flex-start` (default), `flex-end`, `center`, `space-between`, `space-around` | Spacing of child elements along the main axis. |
| `padding` | Integer | Inner padding (all sides) expressed in character cells. |
| `padding-top` \| `padding-right` \| `padding-bottom` \| `padding-left` | Integer | Detailed inner padding for individual sides. |
| `margin` | Integer | Outer margin (all sides) expressed in character cells. |
| `margin-top` \| `margin-right` \| `margin-bottom` \| `margin-left` | Integer | Detailed outer margins for individual sides. |
| `border` | Color | Enables a border around the element with the specified color and default style (`single`). |
| `border-style` | `single`, `double`, `rounded` | The character style used to draw the border (Unicode box drawing characters). |
| `border-color` | Color | Specifies or overrides the border color. |

---

## 3. Defining Colors

In TCSS, colors can be defined in four ways:

| Color Format | Description | Example |
|---|---|---|
| **CSS Color Name** | A basic color name (automatically mapped to TrueColor RGB) | `fg: red;`, `bg: blue;` |
| **Hex RGB** | 7-character hexadecimal string (24-bit TrueColor) | `fg: #ff5500;`, `bg: #0f172a;` |
| **0-255 Number** | Index from the 256-color ANSI palette (useful fallback for older terminals) | `fg: 46;` (bright green), `bg: 234;` (dark gray) |
| **Reset / Transparent** | Restores the default terminal color or makes the background transparent | `fg: reset;`, `bg: transparent;` |

### Supported Named Colors (CSS Color Names)
OxiTerm recognizes exactly the following color names:
* `black` (RGB 0, 0, 0)
* `red` (RGB 255, 0, 0)
* `green` (RGB 0, 255, 0)
* `yellow` (RGB 255, 255, 0)
* `blue` (RGB 0, 0, 255)
* `magenta` (RGB 255, 0, 255)
* `cyan` (RGB 0, 255, 255)
* `white` (RGB 255, 255, 255)

---

## 4. Drawing Borders

Enabling a border on an element is done using the `border` or `border-color` properties. The border occupies **exactly 1 character cell** of width and height on the element's edges.

### Border Styles (`border-style`):
To draw borders, OxiTerm uses Unicode box drawing semigraphics characters:

* **`single`** (default):
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

## 5. What TCSS Does NOT Support (compared to browser CSS)

* **No Units:** Dimensions, margins, and paddings are specified as pure integers (representing terminal character cells). Units such as `px`, `em`, `rem`, `%` are not supported.
* **No `display: none`:** Hiding elements is done entirely at the DOM structure level using the `bind-show` attribute.
* **No Text Formatting:** Properties like `font-size`, `font-family`, `font-weight` (e.g. bold), or `font-style` (e.g. italic) are not supported in TCSS because the terminal enforces a fixed-width monospace font.
* **No Background Corner Rounding:** The `border-radius` property is not supported. The only way to get rounded corners is to use `border-style: rounded;`.
