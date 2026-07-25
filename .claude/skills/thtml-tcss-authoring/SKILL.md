---
name: thtml-tcss-authoring
description: Write or edit THTML markup and TCSS styles for OxiTerm terminal UI applications. Use this skill whenever you touch a .thtml file, write a <style> block, debug a layout that renders wrong (overlapping borders, text spilling past a frame, elements the wrong height, clicks that miss their target), or are asked to build any screen, panel, page, form, or dashboard for OxiTerm — even if the request never says "THTML" or "TCSS". THTML looks like HTML and TCSS looks like CSS, but both are much smaller than they appear and both fail silently, so do not rely on web intuition.
---

# Authoring THTML and TCSS

THTML is a declarative markup language rendered server-side to a fixed character grid. TCSS is its styling language. Both **resemble** HTML/CSS closely enough that the resemblance is the main hazard: web habits produce documents that parse cleanly, render wrong, and give you no error.

Read this whole file before writing markup. The exhaustive reference lives in `docs/thtml-reference.md` and `docs/tcss-reference.md` — consult them for detail, but where this skill and those docs disagree, **this skill is correct** (it was derived from the parser source; the docs contain known errors, listed in §2.2).

---

## 1. The silent-failure rule

This is the single most important fact about TCSS. In `oxiterm-renderer/src/parser/tcss.rs`:

- An **unrecognised property** falls through to `_ => None` and is **discarded without a warning**.
- A **malformed integer value** goes through `value.trim().parse().unwrap_or(0)` and becomes **`0`**.

So `flex: 1`, `overflow: hidden`, `display: flex`, `gap: 2`, `text-align: center`, `font-weight: bold`, `position: absolute` all vanish silently. And `height: auto` becomes `height: 0`, collapsing the element to nothing.

Nothing in the toolchain catches this. `oxiterm check` will report the file as valid. There is no console warning. The only symptom is a layout that looks wrong, which is why layout bugs in this project are historically diagnosed by *reading the parser*, not by reading error output.

**Therefore: only use properties from the table in §2. If you want a property that is not in that table, it does not exist, and you must achieve the effect with explicit integers instead.**

---

## 2. TCSS: the complete property set

These are all of them. There are no others.

| Property | Values | Notes |
|---|---|---|
| `width` | integer (columns) | Omit for content-sizing. |
| `height` | integer (rows) | Omit for content-sizing. **Never write `auto`.** |
| `fg` / `color` | color | Text colour. |
| `bg` / `background-color` | color | Background colour. |
| `flex-direction` | `row` (default), `column` | Any other value silently becomes `row`. |
| `align-items` | `flex-start` (default), `flex-end`, `center`, `stretch` | Cross axis. |
| `justify-content` | `flex-start` (default), `flex-end`, `center`, `space-between`, `space-around` | Main axis. |
| `padding` | integer | All sides. |
| `padding-top`/`-right`/`-bottom`/`-left` | integer | |
| `margin` | integer | All sides. |
| `margin-top`/`-right`/`-bottom`/`-left` | integer | |
| `border` | color | Enables the border, default `single` chars. |
| `border-style` | `single`, `double`, `rounded` | Any other value silently becomes `single`. |
| `border-color` | color | |
| `wrap` | `word` | Anything other than `word` means no wrapping. |

Colors: named (`black red green yellow blue magenta cyan white` — exactly these eight), hex `#rrggbb`, a 0–255 ANSI palette index, or `reset` / `transparent`.

No units anywhere. `10px`, `50%`, `2em` all parse to `0`.

### 2.1 Any border property turns the border on

`border`, `border-color`, and `border-style` each construct the border if it is absent. Setting `border-color: #334155` purely "for the colour" switches the border on and charges you its 2 rows and 2 columns. If you do not want a frame, do not mention borders at all.

### 2.2 Known errors in the shipped documentation and examples

Do not copy these patterns even though they appear in the repository:

- **`flex: 1`** — used 72 times across 34 files in `examples/`, and recommended twice in `docs/thtml-reference.md §7`. It is not a property. It does nothing. Those boxes size to their content rather than absorbing free space.
- **`height: auto`** — recommended in `docs/thtml-reference.md §7.2`. It sets height to `0`.
- `docs/tcss-reference.md §2` omits `wrap` from some contexts and does not flag that unknown properties are dropped silently.

To make a section absorb remaining vertical space, there is no flex-grow equivalent. **Compute the height explicitly** (§3.2).

---

## 3. Layout: the border-box budget

This is where almost every authoring error happens.

### 3.1 Borders cost 2 rows and 2 columns

The engine uses Taffy's default **border-box** sizing. A declared `height` *includes* the border. So:

| Declaration | Border rows | Content rows |
|---|---|---|
| `border-style: single; height: 3` | 2 | 1 |
| `border-style: single; height: 5` | 2 | 3 |
| `border-style: single; height: 1` | 2 | **−1 → borders collide with neighbours** |

**A bordered element cannot be shorter than 3 rows.** The most common single mistake in this codebase is a button class written as `height: 1` because the label is one line; a bordered button occupies **3 rows**. Padding adds on top: `border-style: single; padding-left: 1; padding-right: 1` consumes 4 columns before any character appears.

### 3.2 A container with an explicit height does not grow

If you write `height: 7` and the children need 9 rows, the children overflow past the frame. Nothing warns you. Do the arithmetic:

```
container height = top border (1)
                 + Σ (each child's height + its vertical margins)
                 + bottom border (1)
```

**Worked example** — a footer bar holding a one-row progress line, a 1-row gap, and a row of bordered buttons:

```
border          1
progress row    1
margin-top      1
button row      3   ← bordered, so 3 not 1
border          1
─────────────────
total           7   → height: 7, and not one row less
```

The reverse also applies: a parent's declared height is a hard ceiling for its children's *sum*, and the page root's height is a hard ceiling for the sum of its sections. Overrun makes the page scroll (`PgDn` indicator) instead of fitting.

Because there is no `flex: 1`, sizing a "fill the rest" region means: take the root height, subtract every rigid sibling's height and margins, and write the remainder as an integer. Redo that arithmetic every time you add or resize a sibling.

### 3.3 Text width and wrapping

- Default (`wrap: none`): a `<text>` node's width equals its glyph count and it **never shrinks**. In a narrow container it pushes the parent or overflows the right border. This is deliberate — shrinking would collapse clickable labels to one column and break their hit boxes — but it means long unwrapped text destroys layouts.
- `wrap: word`: breaks **only at spaces**. The longest single token sets the minimum width.
- A long token with no spaces — a URL, a hash, a file path — **will not break**, with or without `wrap: word`. There is no character-level wrapping mode. To display one, either constrain the parent's `width` so the token is visibly clipped, or restructure so the token is not displayed in full.

---

## 4. THTML: the complete tag set

Eight tags. An unknown tag is a **parse error** (this one does fail loudly).

| Tag | Purpose | Notes |
|---|---|---|
| `<screen>` | implicit root | Created by the parser. Never write it. |
| `<box>` | container | Flexbox container, like a `<div>`. |
| `<text>` | text | Text content only, no element children. |
| `<input>` | text field | Needs `bind-value`. |
| `<button>` | focusable button | Text only. |
| `<img>` | image / Lottie | Needs `src` **and** `width` **and** `height`. |
| `<video>` | video | Same requirements; needs `ffmpeg` present. |
| `<for>` | loop | Exactly one template child; `each` names a List state key; `{item}` in the child's text is substituted. |

**Any** tag may self-close — the parser's self-closing branch (`thtml.rs:225`) is not restricted by tag name, so `<text bind-state="k"/>` is valid. `docs/thtml-reference.md` claims only `<img>` and `<input>` may do this; that restriction is not in the code. A self-closed tag has no children and no text, so self-closing `<text>` is only useful with `bind-state`, and self-closing `<for>` is always a mistake.

`<img>`/`<video>` without `src`, or with `width`/`height` missing or zero, is a **hard parse error** with a message naming what is missing. This is intentional — the grid cannot lay out an unknown-size box.

### 4.1 Universal attributes

`id`, `class`, `style`, `event-htmx`, `bind-state`, `bind-show`.

`event-htmx` may hold several actions separated by `;` or `,`, executed left to right:

| Form | Effect |
|---|---|
| `inc:key` / `dec:key` | ±1 on an Int |
| `toggle:key` | flip a Bool |
| `set:key=value` | set a Str |
| `append:key=value` | push onto a List |
| `clear:key` | reset to the type's zero value — **no-op if the key does not yet exist** |
| `open:URL` | open in the system browser; **web sessions only**, silently ignored over SSH; `http`/`https` only |
| `something.thtml` | navigate to another page, preserving all session state |
| `any_other_string` | dispatched to the App Server (see the `oxiterm-app-integration` skill) |

An important consequence: a non-`open:`, non-`.thtml` action is applied locally **and** dispatched to the App Server. Built-in actions like `set:tab=x` reach the App Server too.

### 4.2 `bind-show`

Hidden nodes are removed from layout entirely — they occupy no rows.

| Form | True when |
|---|---|
| `bind-show="key"` | truthy: `true`, non-zero Int, non-empty Str other than `"false"`, non-empty List |
| `bind-show="key=value"` | the value stringifies to `value`, or the List contains it |
| `bind-show="key=true"` | truthy; **false if the key is absent** |
| `bind-show="key=false"` | falsy **or the key is absent** |

A missing key makes every form false except `key=false`. So `bind-show="logged_in=false"` is the way to make an element visible before any state has been set — the standard idiom for a login screen that must appear on first paint.

### 4.3 Reserved state keys

Keys beginning with `_` are engine-owned; App Server patches that target them are rejected with a log warning. Read them, never write them.

| Key | Value |
|---|---|
| `_username` | authenticated username |
| `_auth_method` | `SshKey`, `SshPassword`, `TrustedHeader`, `Guest` |
| `_is_web` | `"true"` for browser sessions, `"false"` for SSH |

`_is_web` is how you branch on transport, which matters because the two transports have genuinely different capabilities (§5).

---

## 5. Web and SSH are not the same target

The same document renders to a browser canvas and to a terminal, and they differ in ways that change design decisions:

- **`open:` works only on web.** Over SSH it is ignored.
- **The web canvas has no text selection.** A user cannot select or copy text from it. Anything the user must copy — a URL, a token, a code — has to be *clickable* on web. On SSH the terminal's own selection handles copying, so plain text is fine there.
- Emoji and CJK occupy two columns; the layout accounts for this, but see §6.

Use `bind-show="_is_web=true"` / `="_is_web=false"` to give each transport the affordance it can actually support, rather than picking one and degrading the other.

### 5.1 Mobile variants

`pathsafe::resolve_variant` looks for `<stem>_mobile.thtml` beside the requested file and serves it to mobile viewports, falling back to the base file when absent. If you add a page and mobile matters, add the `_mobile` sibling; every page in `examples/` has one.

---

## 6. Glyph choice in clickable labels

Ambiguous-width characters — `←` `→` `×` and the box-drawing set — count as one column in the layout model but may render two columns wide in a client font. When one sits in a clickable label, the visible text shifts off its hit box and **clicks miss**.

Use ASCII for anything interactive: `<` not `←`, `>` not `→`, `x` not `×`. The focus-ring markers in the engine are ASCII `>` and `<` for exactly this reason.

`oxiterm check` emits a warning when it finds ambiguous-width characters in a clickable label. Read the warnings; do not just check the exit code.

---

## 7. Verification before you claim it works

`oxiterm check <file>` validates parse structure: tag names, nesting, closing tags, and the `<img>`/`<video>` size requirement. It also warns about ambiguous-width clickables.

**It does not validate layout at all.** It will happily pass a file with `height: 1` on a bordered box, a container too small for its children, `flex: 1` on every section, and `height: auto` collapsing an element to nothing. A clean `check` says the document parses, not that it renders.

So after `check` passes, do this by hand:

1. **Re-derive every explicit height.** For each container with a declared `height`, sum its children's heights plus vertical margins plus 2 if it is bordered. Compare. Fix mismatches.
2. **Grep your own diff for phantom properties.** `grep -nE '(flex|display|gap|overflow|position|font-|text-align|border-radius|line-height)\s*:' <file>` should return nothing.
3. **Grep for `height: auto` and `width: auto`.** Should return nothing.
4. **Check every bordered element is at least 3 rows tall.**
5. **Confirm every `<text>` that could be long has either `wrap: word` plus a bounded parent width, or a guarantee of being short.**
6. **Look at it.** Render the page and actually view it, over both transports if the change touches interaction. Every significant layout bug in this project's history was found by a human clicking or looking, not by a passing test: reattach rendering, pixel hairlines, collapsed hit boxes, the one-column `<text>`. A test suite that passes is not evidence the screen is right.

If you cannot render it, say so plainly rather than implying the layout is verified.

---

## 8. A correct skeleton

```html
<style>
  /* bordered button: 3 rows, never 1 */
  .btn { border-style: single; border-color: #4ade80;
         height: 3; padding-left: 1; padding-right: 1;
         align-items: center; justify-content: center; }
  .row1 { height: 1; }
  .muted { fg: #94a3b8; height: 1; }
</style>

<!-- root: 24 rows = header 3 + body 18 + footer 3 -->
<box style="flex-direction: column; width: 80; height: 24; bg: #0f172a;">

  <box style="height: 3; border-style: single; border-color: #334155;
              align-items: center; justify-content: space-between;
              padding-left: 1; padding-right: 1;">
    <text style="fg: #4ade80; height: 1;">Panel</text>
    <!-- ASCII '<', not '←' — this is clickable -->
    <text class="muted" event-htmx="index.thtml">&lt; Back</text>
  </box>

  <!-- body: no border, padding 1 → 16 content rows.
       used: text 1 + margin 1 + btn 3 = 5. slack 11. -->
  <box style="height: 18; flex-direction: column; padding: 1;">
    <text bind-state="status" class="row1"/>
    <box class="btn" style="margin-top: 1;" event-htmx="refresh">
      <text style="fg: #4ade80; height: 1;">Refresh</text>
    </box>
  </box>

  <box style="height: 3; border-style: single; border-color: #334155;
              align-items: center; padding-left: 1;">
    <text class="muted">[Tab] move  [Enter] activate  [Q] quit</text>
  </box>

</box>
```

Every height in that file is an integer someone can add up, which is the whole discipline.
