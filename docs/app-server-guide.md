# Integration with an External Application Server (App Server)

OxiTerm is responsible for the user interface layer (THTML/TCSS), handling terminal events (keyboard/mouse), and managing basic session state (`StateManager`). However, for operations that go beyond built-in actions — such as form validation, database queries, or external API integration — you must connect an external application server (**App Server**).

---

## 1. Communication Protocol

Communication between OxiTerm and the external server takes place using HTTP POST requests. On every `event-htmx` event, OxiTerm sends an asynchronous request with a JSON payload to the URL defined in the `OXITERM_APP_SERVER` environment variable.

### JSON Payload Schema (OxiTerm → App Server)

```json
{
  "action": "validate_form",
  "state": {
    "email": "user@example.com",
    "step": "2"
  },
  "session_id": 42
}
```

### Field Descriptions

| Field | Type | Description |
|---|---|---|
| `action` | string | The raw value of the `event-htmx` attribute (e.g. `"login"`, `"save_record"`) that triggered the event. |
| `state` | object | A key-value dictionary representing the current session state. All state values are sent as strings. |
| `session_id` | integer | The unique identifier assigned to the given SSH or WebSocket session. Used to distinguish between individual connected users. |

> [!WARNING]
> Event propagation by OxiTerm operates in a **fire-and-forget** mode. The OxiTerm server does not wait for an HTTP response from the application server before re-rendering the screen. The response from the application server should have a status code of `204 No Content` or `200 OK` with an empty body.

---

## 2. Python Implementation

### Flask (simple example)
```bash
pip install flask
```

```python
from flask import Flask, request
app = Flask(__name__)

@app.route("/events", methods=["POST"])
def handle_event():
    payload    = request.json
    action     = payload["action"]
    state      = payload["state"]
    session_id = payload["session_id"]

    if action == "validate_form":
        email = state.get("email", "")
        if "@" not in email:
            print(f"Session {session_id}: Invalid email format")

    elif action == "save_record":
        print(f"Saving record for session {session_id}: {state}")

    return "", 204

if __name__ == "__main__":
    app.run(port=3000)
```

### FastAPI (asynchronous server with Pydantic)
```bash
pip install fastapi uvicorn
```

```python
from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI()

class OxiEvent(BaseModel):
    action: str
    state: dict[str, str]
    session_id: int

@app.post("/events")
async def handle(ev: OxiEvent):
    match ev.action:
        case "validate_form":
            await validate_email(ev.session_id, ev.state)
        case "save_record":
            await save_to_db(ev.session_id, ev.state)
    return {}

async def validate_email(session_id: int, state: dict[str, str]):
    # Validation logic...
    pass

async def save_to_db(session_id: int, state: dict[str, str]):
    # DB Save...
    pass

# Run with: uvicorn server:app --port 3000
```

---

## 3. Node.js Implementation (JavaScript / TypeScript)

### Express
```bash
npm install express
```

```javascript
const express = require("express");
const app = express();
app.use(express.json());

app.post("/events", (req, res) => {
  const { action, state, session_id } = req.body;

  switch (action) {
    case "login":
      if (state.username === "admin" && state.password === "secret") {
        console.log(`Session ${session_id}: authentication succeeded`);
      }
      break;

    case "fetch_data":
      fetchFromDB(state).then(data => {
        console.log(`Session ${session_id}: data fetched`, data);
      });
      break;
  }
  res.status(204).send();
});

app.listen(3000);
```

### Hono (Bun / Edge)
```javascript
import { Hono } from "hono"
const app = new Hono()

app.post("/events", async c => {
  const { action, state, session_id } = await c.req.json()
  console.log(`[Session ${session_id}] Event: ${action}`, state)
  return c.body(null, 204)
})

export default { port: 3000, fetch: app.fetch }
```

---

## 4. Rust Implementation (Axum)

Running your application server in Rust allows you to share event structure types (e.g., through a shared crate in a workspace).

```toml
[dependencies]
axum       = "0.7"
tokio      = { version = "1", features = ["full"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
```

```rust
use axum::{Router, Json, http::StatusCode, routing::post};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct OxiEvent {
    action:     String,
    state:      HashMap<String, String>,
    session_id: usize,
}

async fn handle(Json(ev): Json<OxiEvent>) -> StatusCode {
    match ev.action.as_str() {
        "save_config" => {
            let k = ev.state.get("config_key").cloned().unwrap_or_default();
            let v = ev.state.get("config_val").cloned().unwrap_or_default();
            println!("[Session {}] Saved: {}={}", ev.session_id, k, v);
        }
        "run_job" => {
            tokio::spawn(async move {
                // Execute long-running background task...
            });
        }
        _ => {}
    }
    StatusCode::NO_CONTENT
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/events", post(handle));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

---

## 5. Complete Flow (Login Form Example)

Here is how to construct a THTML interface to pass input values to your application server:

```html
<style>
  .field { border-style: single; border-color: #334155;
           height: 1; padding-left: 1; margin-top: 1; }
  .btn   { border-style: single; border-color: #4ade80;
           padding: 1; height: 3; align-items: center; }
</style>

<box style="flex-direction: column; width: 60; height: 24; bg: #0f172a;">
  <text style="fg: #38bdf8; height: 1; margin-bottom: 2;">Login Panel</text>

  <text style="fg: #94a3b8; height: 1;">Username:</text>
  <input bind-value="username" class="field" placeholder="Enter username..."/>

  <text style="fg: #94a3b8; height: 1; margin-top: 1;">Password:</text>
  <input bind-value="password" class="field" placeholder="••••••"/>

  <!-- Clicking will trigger the "login" action -->
  <box class="btn" style="margin-top: 2;" event-htmx="login">
    <text style="fg: #4ade80; height: 1;">Log In</text>
  </box>
</box>
```

When the user fills in the fields and clicks the "Log In" button, OxiTerm will send the following POST request to the App Server:
```json
{
  "action": "login",
  "state": {
    "username": "admin",
    "password": "mysecretpassword"
  },
  "session_id": 1
}
```

---

## 6. Concurrency and Multi-Session Patterns

When the OxiTerm server handles multiple users concurrently, the application server must properly distinguish between sessions:
* **User Identification:** Always use the `session_id` field as the unique key to identify the current connection. You can map `session_id` to a logged-in user identifier in a cache (e.g. in Redis or an in-memory HashMap).
* **State Isolation:** OxiTerm automatically isolates and maintains a separate `StateManager` state for each connected SSH/Web session. Your App Server should also maintain complete data isolation between different `session_id` values.
