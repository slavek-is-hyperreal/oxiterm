# Guide: Creating Applications with OxiTerm

OxiTerm is a framework for creating server-side rendered TUI (Terminal User Interface) applications. You describe the interface declaratively using the **THTML** markup language and style it using **TCSS**. The OxiTerm server parses these files, computes the layout using Flexbox, and then sends minimal ANSI instructions (diffs) to the client via SSH or WebSocket. The client only needs a standard terminal — no dedicated software needs to be installed.

---

## 1. Installation

A Rust environment is required to build OxiTerm from source. If you don't have it, install it from: [https://rustup.rs](https://rustup.rs)

Then clone the repository and build the project:
```bash
git clone https://github.com/slavek-is-hyperreal/oxiterm && cd oxiterm
cargo build --release
```
You can find the compiled executable in: `target/release/oxiterm`

---

## 2. Quick Start (First Application)

Create a file named `hello.thtml` with the following content:

```html
<box style="flex-direction: column; width: 80; height: 24; bg: #0f172a;">
  <box style="height: 3; bg: #1e293b; align-items: center;
              padding-left: 2; border-style: single; border-color: #334155;">
    <text style="fg: #38bdf8; height: 1;">Welcome to OxiTerm!</text>
  </box>
  <box style="padding: 2;">
    <text style="fg: #e2e8f0; height: 1;">This is a server-side rendered TUI application.</text>
  </box>
</box>
```

Start the server in development mode (without authentication):
```bash
oxiterm serve hello.thtml --port 2222 --no-auth
```

Now you can connect to the application from any terminal using SSH:
```bash
ssh localhost -p 2222
```

---

## 3. Hot Reload

OxiTerm automatically monitors the served `.thtml` file. After editing and saving the file to disk, all active SSH and Web connection sessions will be **instantly updated** without needing to reconnect. Crucially, the application state (stored in `StateManager`) is fully preserved during layout reloads!

---

## 4. Adding State and Actions

You manage state and interaction using `bind-state` and `event-htmx` attributes:

```html
<box style="flex-direction: column; width: 80; height: 24; bg: #0f172a;">
  <!-- bind-state binds text content to the state key "n" -->
  <text bind-state="n" style="fg: #fbbf24; height: 1; margin: 2;">0</text>
  
  <box style="flex-direction: row; padding-left: 2;">
    <!-- event-htmx defines the action when clicked or submitted with Enter -->
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

## 5. CSS Classes

To avoid repeating inline styles, you can define a `<style>` block containing TCSS rules:

```html
<style>
  .card  { border-style: rounded; padding: 1; flex-direction: column; }
  .blue  { border-color: #38bdf8; }
  .muted { fg: #64748b; height: 1; }
</style>

<box class="card blue" style="height: 8;">
  <text style="fg: #38bdf8; height: 1;">Blue Card</text>
  <text class="muted">Sample card content.</text>
</box>
```

---

## 6. Tabs Using `bind-show`

The `bind-show` attribute allows you to conditionally hide or show elements based on state:

```html
<box style="flex-direction: row; height: 3;">
  <box event-htmx="set:tab=info" style="border-style: single; padding: 1; height: 3;">
    <text style="height: 1;">Info</text>
  </box>
  <box event-htmx="set:tab=logs" style="border-style: single; padding: 1; height: 3;">
    <text style="height: 1;">Logs</text>
  </box>
</box>

<!-- tab=false means the element is visible when the key "tab" is absent in state (visible by default) -->
<box bind-show="tab=false" style="padding: 2;">
  <text style="height: 1;">Info tab content.</text>
</box>

<box bind-show="tab=logs" style="padding: 2;">
  <text style="height: 1;">Logs tab content.</text>
</box>
```

---

## 7. Text Input Fields (Input)

The `<input>` tag allows you to collect data typed by the user:

```html
<input bind-value="query" placeholder="Search..."
       style="height: 1; border-style: single; border-color: #38bdf8;"/>
<text bind-state="query" style="fg: #94a3b8; height: 1;"/>
```

* Use the `Tab` key to focus on the `<input>` element.
* Start typing — the Predictive Echo input buffer will immediately update the view (locally).
* The `Backspace` key deletes the last character.
* The `Enter` key commits the typed text and saves it in the state under the key defined in `bind-value` (e.g. `"query"`), and also triggers the `event-htmx` action if defined.

---

## 8. Images and Media

OxiTerm enables directly embedding vector graphics, animations, and video files:

```html
<img src="logo.svg"    alt="Project logo" style="width: 20; height: 10;"/>
<img src="bell.json"   alt="Vector bell animation" style="width: 12; height: 6;"/>
<video src="clip.mp4"  alt="Video presentation" style="width: 40; height: 20;"/>
```

* **Automatic Protocol Detection:** OxiTerm dynamically matches the graphic format to your terminal capabilities (Kitty Graphics Protocol → Sixel → Unicode character blocks `▀▄█`).
* **Formats:** SVG, PNG, JPG images and Lottie animations (`.json`) are supported.
* **Video:** Video playback requires the `ffmpeg` tool to be present in your environment's `PATH`.

---

## 9. Key Bindings

| Key | Action |
|---------|-----------|
| `Tab` / `↓` / `→` | Go to the next interactive element (focus) |
| `Shift + Tab` / `↑` / `←` | Return to the previous element |
| `Enter` | Activate the focused element |
| `PgUp` / `b` | Scroll view one page up |
| `PgDn` / `Space` | Scroll view one page down |
| `Q` / `q` | Close session and disconnect |

---

## 10. Page Navigation

You can switch entire screens by pointing to a `.thtml` file in the `event-htmx` action:

```html
<text event-htmx="settings.thtml" style="height: 1;">→ Go to settings</text>
```

> [!TIP]
> The session state (`StateManager`) is shared and **preserved** during navigation between `.thtml` pages. Variables set on one page will be available on the newly loaded page.

---

## 11. Web Session Management & Reconnection

When running OxiTerm applications over WebSockets (via browser clients), the platform provides robust connection and session persistence features:

* **Session Reattachment:** OxiTerm sessions persist on the server even if the network connection drops. Reconnecting to a session is done automatically by presenting the session token in the connection URL (`?session=<token>`).
* **Clean URLs:** Upon connection, OxiTerm extracts the `session` token, saves it to the browser's `sessionStorage`, and removes it from the address bar. The visible URL will only contain page parameters (e.g. `?page=settings.thtml`), preventing session tokens from being accidentally shared or leaked when copying URLs.
* **Auto-Reconnection:** If the WebSocket connection is temporarily lost, the client attempts to reconnect using an exponential backoff algorithm capped at 8 seconds.
* **Connection Takeover:** If a session token is loaded in a new tab or browser window, the new connection takes over the session. The old connection is notified via a `0xFF` byte and closed automatically, immediately disabling its reconnection loop to prevent loop-fighting over the socket channel.
* **Reattach Page Sync:** Reattaching to a session with a different `page` parameter (e.g., opening a shared link like `?page=other.thtml`) will navigate the session to that new page.

---

## 12. Integration with an External Application Server (App Server)

If your application requires complex business logic, database access, or external APIs, you can connect an external application server.

Run OxiTerm with the `OXITERM_APP_SERVER` variable:
```bash
OXITERM_APP_SERVER=http://localhost:3000/events oxiterm serve app.thtml
```

For every user action (`event-htmx`), OxiTerm will dispatch an asynchronous POST request in the background with the current state:
```json
{
  "action": "save_profile",
  "state": { "username": "admin", "email": "user@example.com" },
  "session_id": 42
}
```

The App Server can return a `200 OK` response with a JSON object to apply a **State Patch** back to the active session state. If no state changes are required, it can return a `204 No Content` status.

For more information, see [app-server-guide.md](app-server-guide.md).

---

## 13. Mobile Support & Responsive Design

OxiTerm features built-in support for responsive mobile interfaces using a unified web client page at `/`.

### Responsive Viewport Detection
Rather than using server-side redirects or cookies, the mobile layout selection operates dynamically:
* **Viewport Check:** On load, the web client detects the screen width. If the width is under `800px`, the client classifies the session as mobile.
* **Viewport Synchronization:** The client sends a `0x11` viewport configuration message to the server immediately after connecting, indicating the mobile status (`1` for mobile, `0` for desktop).
* **Dynamic Resizing:** If the client window size is resized across the `800px` threshold, the client sends another `0x11` message to notify the server of the viewport change.
* **Server-Side Hot-Reload:** Upon receiving the `0x11` message, the server triggers a `SwitchViewport` event, updates the session's mobile status flag (`is_mobile`), and reloads the current page.

To build a mobile-optimized view:
1. **Naming Convention:** Create a duplicate of your THTML file and add the `_mobile.thtml` suffix. For example, if your desktop file is `dashboard.thtml`, name the mobile file `dashboard_mobile.thtml`.
2. **Dynamic Fallback:** If a user accesses your app on a mobile device and `dashboard_mobile.thtml` exists, OxiTerm will automatically load it. If not, it will fallback to `dashboard.thtml`.
3. **Optimized Dimensions:** While desktop layouts typically use `80x24` grid dimensions, mobile layouts are optimized for a `48x30` grid to fit neatly on phone screens and provide comfortable tap target sizes for buttons.

---

## 14. Environment Variables

You can configure the server behavior using the following environment variables:

| Variable | Default | Description |
|---|---|---|
| `OXITERM_PORT` | `2222` | Port on which the SSH server listens |
| `OXITERM_HOST` | `0.0.0.0` | SSH listening IP address |
| `OXITERM_PASSWORD` | (none) | SSH login password (if set, enables password authentication) |
| `OXITERM_NO_AUTH` | `false` | Disables SSH authentication (required in dev mode when authorized_keys are missing) |
| `OXITERM_WEB_PORT` | `8080` | HTTP/WebSocket server port for web browser access |
| `OXITERM_APP_SERVER` | (none) | URL of the external application server for event-htmx actions |
| `RUST_LOG` | `warn` | Server logging level (e.g. `debug`, `info`, `warn`, `error`) |

