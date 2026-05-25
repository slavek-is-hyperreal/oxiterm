# THTML — Terminal HTML Language Reference

THTML (Terminal HTML) is OxiTerm's declarative markup language used to build server-side rendered TUI (Terminal User Interface) applications. It is a subset of XML with a strict tag structure and a small, specialized dictionary of attributes.

---

## 1. Document Structure

Every THTML document is parsed into a tree of nodes, where the default, implicit root is the `<screen>` node. An optional `<style>` block containing TCSS styling rule definitions can appear anywhere in the document (usually at the beginning).

```html
<!-- Optional style block -->
<style>
  .btn { border-style: single; border-color: #4ade80; }
  .header-text { fg: #38bdf8; height: 1; }
</style>

<!-- UI components tree -->
<box style="flex-direction: column; width: 80; height: 24; bg: #0f172a;">
  <text class="header-text">Welcome to OxiTerm!</text>
</box>
```

Files must not contain elements specific to browser HTML, such as `<!DOCTYPE>`, `<html>`, `<head>`, or `<body>`.

---

## 2. Supported Tags

OxiTerm supports exactly seven tags. Using an unknown tag will cause a document parsing error.

| Tag | Role | Children | Notes |
|---|---|---|---|
| `<screen>` | Implicit root | Any | Created automatically by the parser, never written directly. |
| `<box>` | Layout container | Any | Equivalent to HTML `<div>`. Operates as a Flexbox container by default. |
| `<text>` | Inline text | None (text only) | Displays text content. Supports Unicode, double-width characters (CJK), and emojis. |
| `<input>` | Text field | None | Used to collect user input. Requires the `bind-value` attribute. |
| `<button>` | Interactive button | None (text only) | Focusable keyboard node; triggers the action defined in `event-htmx`. |
| `<img>` | Image or animation | None | Displays SVG, PNG, JPG files and Lottie animations (`.json`). |
| `<video>` | Video player | None | Play video files smoothly in the background. Requires `ffmpeg` in the system. |

> [!WARNING]
> The `<img>` and `<input>` tags can be self-closing (`<img />`, `<input />`). All other tags absolutely require closing tags (e.g., `<text>Content</text>`).

---

## 3. Universal Attributes

The following attributes can be applied to any node type:

| Attribute | Value Type | Description |
|---|---|---|
| `id` | Text | Unique identifier of the node in the tree. Used by the CSS `#id` selector. |
| `class` | Text (space-separated) | TCSS style classes. Applied in the order they appear in the stylesheet. |
| `style` | TCSS Style | Inline style declarations (semicolon-separated). Have the highest priority in the cascade. |
| `event-htmx` | Action text | Action (or sequence of actions) triggered when the element is clicked or when `Enter` is pressed. |
| `bind-state` | State key | Reactively displays the current value of the associated key from `StateManager` as text content. |
| `bind-show` | Condition text | Conditionally hides/shows the node. Hidden nodes are completely removed from the layout. |

---

## 4. Tag-Specific Attributes

| Tag | Attribute | Description |
|---|---|---|
| `<img>`, `<video>` | `src` | Relative path to the resource file (relative to the directory containing the `.thtml` file). Paths are checked for Path Traversal attack attempts. |
| `<img>`, `<video>` | `alt` | Textual description of the media element, used in accessibility mode (`--a11y`). |
| `<input>` | `placeholder` | Helper text displayed in the field when the input buffer is empty. |
| `<input>` | `name` | Machine name of the input field, used as a label in the Accessibility Tree. |
| `<input>` | `bind-value` | State key in `StateManager` where the typed text is saved **in real-time** (on every keystroke). |

---

## 5. `event-htmx` Actions

The `event-htmx` attribute supports a single action or a sequence of actions. Instructions are executed from left to right. Both **semicolon** (`;`) and **comma** (`,`) can be used as instruction separators.

| Action Format | Effect | Example |
|---|---|---|
| `inc:key` | Increments the integer state (Int) value by 1 | `event-htmx="inc:counter"` |
| `dec:key` | Decrements the integer state (Int) value by 1 | `event-htmx="dec:counter"` |
| `toggle:key` | Flips the boolean state (Bool) value | `event-htmx="toggle:sidebar_open"` |
| `set:key=value` | Sets the state (Str) to the specified text | `event-htmx="set:tab=settings"` |
| `append:key=value` | Appends the specified text value to a list state (List) | `event-htmx="append:logs=new_event"` |
| `clear:key` | Resets state to default value for its type (`0`, `false`, `""` or `[]`) | `event-htmx="clear:counter"` |
| `file.thtml` | Switches the current application screen to another `.thtml` file | `event-htmx="dashboard.thtml"` |
| `action1;action2` | Executes multiple actions sequentially | `event-htmx="set:tab=x;inc:views"` |

> [!NOTE]
> Navigating to another `.thtml` page **preserves** all state collected in the session `StateManager` cache. This ensures users do not lose their inputs or session context when transitioning between screens.

---

## 6. `bind-show` Conditions

The `bind-show` attribute is used to reactively hide interface elements. Nodes whose conditions evaluate to `false` are completely excluded from Taffy layout calculations (they consume no space in the terminal).

| Condition Syntax | Condition is `true` when: |
|---|---|
| `bind-show="key"` | The associated key has a boolean value of `true`, a non-zero integer (Int != 0), a non-empty string (Str != "" and Str != "false"), or a non-empty list. |
| `bind-show="key=value"` | The text representation of the state value under this key is exactly equal to `"value"`. If the state value is a `List`, the condition is satisfied if the list contains the element `"value"`. |
| `bind-show="key=false"` | The associated key value is falsy or **the key does not exist in the state**. |
| `bind-show="key=true"` | The associated key value is truthy. If the key does not exist, `false` is returned. |

> [!IMPORTANT]
> If a state key does not exist in the session cache, all condition formats (except `key=false`) evaluate to `false` by default. Using `bind-show="menu=false"` allows a panel to be visible by default at session startup, before the state is initialized by any actions.

---

## 7. State Types

`StateManager` stores typed state values. The table below illustrates how different types are rendered on screen by the `bind-state` attribute:

| State Type | Internal Rust Type | Representation by `bind-state` | Example Output |
|---|---|---|---|
| **Int** | `i64` | Numeric notation | `42` |
| **Bool** | `bool` | Word form: `true` or `false` | `true` |
| **Str** | `String` | Raw string contents | `Hello world` |
| **List** | `Vec<String>` | List of values enclosed in square brackets | `[item1, item2, item3]` |

---

## 8. Interacting with `<input>` Fields

The data entry flow in input fields is as follows:
1. The user moves focus to the `<input>` field using the `Tab` key or arrow keys.
2. The user starts typing. Character by character, the input content is **instantly saved** in the state under the key defined in the `bind-value` attribute. Other elements bound to the same key via `bind-state` will react in real-time to each keystroke.
3. The `Backspace` key deletes the last character from the buffer and updates the state.
4. The `Enter` key commits the field and triggers the action defined in the `event-htmx` attribute (if any).

Example binding input with reactive text:
```html
<input bind-value="email" placeholder="Enter your email..."
       style="height: 1; border-style: single; border-color: #38bdf8;"/>
<text bind-state="email" style="fg: #94a3b8; height: 1;"/>
```

---

## 9. Comments

Standard HTML comments are completely skipped when parsing the document tree:
```html
<!-- This comment will be stripped and will not end up in the DOM tree -->
```

---

## 10. Data Safety and Sanitization

OxiTerm ensures that input provided by THTML files and user interaction does not disrupt terminal output or lead to rendering errors:
* **Styles (`style="..."`):** Any ANSI escape sequences are detected and stripped using regex. This prevents injection attacks with destructive terminal control codes.
* **Action Attributes (`event-htmx`, `bind-state`, `bind-show`):** All control characters and symbols like brackets, quotes, and backslashes (`(`, `)`, `'`, `"`, `<`, `>`, `\`, `` ` ``) are removed. Only alphanumeric characters, spaces, equal signs, and URL-safe characters are allowed.
