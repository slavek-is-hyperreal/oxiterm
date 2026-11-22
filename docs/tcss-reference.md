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

### Render-Mode Conditions (`@media (render: ...)`)

Rules can be restricted to one render backend with a media block in the `<style>` section:

```css
.card { border-style: rounded; padding: 1; }

@media (render: gui) {
  .card { border-radius: 8px; box-shadow: 0 4px 12px rgba(0,0,0,.3); }
}
@media (render: tui) {
  .card { bg: #1a1b26; }
}
```

* **Modes:** `tui` (terminal cells, the default for `THTMLParser::parse`) and `gui` (`THTMLParser::parse_with_mode(.., RenderMode::Gui)`). Whitespace and case around `render`, `:` and the value are ignored.
* **Cascade:** rules inside a block keep their source position, so an unconditional rule before or after the block wins or loses by the usual order-of-appearance rule.
* **Unknown queries are skipped:** any other query (`screen`, `(render: hologram)`, `(render: tui) and ...`) skips the whole block silently and parsing continues after its closing `}`.
* **No nesting:** `@media` inside `@media` is not supported.
* **GUI-only properties:** `border-radius` is parsed (see below) but only display-list (GUI) clients draw it; `box-shadow`, `font-family` and `font-size` are ignored like any unknown property. They do not affect the other declarations of the same rule, even when the value contains commas, parentheses or quotes.

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
| `border-radius` | Length >= 0 (cells, `px`, `dp`) | GUI only: corner radius of the fill and border (see the `@media` section). Ignored in the terminal. |
| `flex` | Number > 0 (e.g. `1`, `2.5`) | Flex shorthand. Sets `flex-grow` factor, `flex-shrink: 1.0`, and `flex-basis: 0`. Expands element along container's main axis. |
| `wrap` | `word` | Enables word-wrapping of `<text>` content to the element's width. With `wrap: word` (and a constrained width) the text flows onto multiple rows at word boundaries; without it text stays on a single row. |
| `position` | `static` (default), `relative`, `absolute`, `fixed` | Positioning model. `absolute` removes element from normal flow, positioned relative to nearest positioned ancestor. `fixed` positions relative to viewport origin and does not expand scroll range. |
| `top` \| `right` \| `bottom` \| `left` | Integer | Offset in character cells used with positioned elements. |
| `z-index` | Integer | Stacking context layer order. Higher values are painted on top of lower values and take precedence in mouse hit-testing. |
| `opacity` | Decimal `0.0` - `1.0` | Opacity factor of element. |
| `transition` | `<prop> <duration> [easing] [delay]` | Declares smooth animated transitions on property changes (e.g. `transition: width 300ms ease-out, left 200ms spring(120, 12, 1);`). |

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

---

## 5. Positioning and Stacking Contexts (`position`, `z-index`)

OxiTerm supports standard CSS positioning paradigms:

* **`position: static`** (default): Element participates in normal flexbox layout flow.
* **`position: relative`**: Element participates in flex layout and acts as a positioned reference container for absolute descendants.
* **`position: absolute`**: Element is taken completely out of document flow and positioned relative to the nearest positioned ancestor (or the screen root). Insets (`top`, `left`, `right`, `bottom`) define coordinates in character cells.
* **`position: fixed`**: Element is positioned directly relative to the terminal viewport and is excluded from the document scroll height calculation (preventing fixed desktop bars or headers from causing scroll overflow).

### Layering and Hit-Testing (`z-index`)
* Elements with a higher `z-index` are drawn on top of elements with a lower `z-index`.
* **Hit-Testing Invariant:** Clicks and pointer interactions are evaluated in reverse visual paint order (`paint_order.rev()`). This ensures that floating windows, popups, and dropdown menus with high `z-index` receive clicks first, even when covering elements underneath.

---

## 6. CSS Transitions & Timing Curves

TCSS supports smooth animated transitions when animatable style properties change:

```css
.window {
    position: absolute;
    width: 30;
    left: 10;
    transition: width 300ms ease-out, left 200ms spring(120, 12, 1);
}
```

### Syntax
`transition: <property> <duration> [timing-function] [delay];`
Multiple properties can be comma-separated.

### Animatable Properties
* `width`, `height`
* `top`, `left`, `right`, `bottom`
* `margin-left`, `margin-top`, `margin-right`, `margin-bottom`, `margin`
* `opacity`
* `all` (animates all supported properties)

### Supported Timing Functions (Easing)
* `linear`: Constant speed.
* `ease` (default): Fast start, smooth deceleration.
* `ease-in`: Slow start.
* `ease-out`: Fast start with deceleration.
* `ease-in-out`: Slow start and slow deceleration.
* `cubic-bezier(x1, y1, x2, y2)`: Custom Bézier timing curve solved via Newton-Raphson iteration.
* `spring(stiffness, damping, mass)`: Physical damped harmonic oscillator simulation.

### Runtime Architecture
When any transition is active, the OxiTerm event loop ticks at 60 FPS (16ms). When all transitions and animations settle and no pointer drag is active, the event loop returns to idle sleep (0% CPU).

---

## 7. Media Element Constraints

* **Media Min Size Floor:** Media elements (`<img>` and `<video>`) enforce a minimum size (`min_size`) based on their declared `width` and `height` attributes. Even when placed inside a flex container or subject to flex layout, media elements will not shrink below their declared dimensions.

---

## 8. What TCSS Does NOT Support & Unknown Property Handling

* **No Units or `auto` Keywords:** Dimensions, margins, and paddings are specified as pure positive integers. Units such as `em`, `%` and keywords like `auto` (e.g. `height: auto`) are not supported. Omitting a dimension property leaves it unconstrained (`Dimension::Auto`).
* **`border-radius` (GUI only):** one non-negative length (cells, or `px`/`dp` through the provisional 8x16 cell, so `16px` = 2 cells; measured along the horizontal axis). It is carried to the display list as the `radius` of that node's fill and border. Terminals ignore it completely; the rule `.card { border-radius: 8px; }` inside `@media (render: gui)` is the intended use. Multi-value forms (`4px 8px`) and negative values are rejected with a warning.
* **Unknown Property Warnings:** Any unrecognized property name (e.g. `font-weight`, `font-style`) or invalid property value (e.g. `height: auto`, `flex: 0`, `flex: -1`) emits a `tracing::warn!` log and is ignored by the parser. The GUI-only properties `box-shadow`, `font-family` and `font-size` are ignored without a warning.
* **No `display: none`:** Hiding elements is done entirely at the DOM structure level using the `bind-show` attribute.
* **No Font Sizing/Families:** Properties like `font-size` or `font-family` are not supported because the terminal enforces a fixed-width monospace font.
* **No Corner Rounding in TUI:** `border-radius` has no effect in the terminal; the only way to get rounded corners there is `border-style: rounded;`.
