# Integration with an External Application Server (App Server)

OxiTerm is responsible for the user interface layer (THTML/TCSS), handling terminal events (keyboard/mouse), and managing basic session state (`StateManager`). However, for operations that go beyond built-in actions — such as form validation, database queries, or external API integration — you must connect an external application server (**App Server**).

---

## 1. Communication Protocol

Communication between OxiTerm and the external server takes place using HTTP POST requests. On every `event-htmx` event, OxiTerm sends an asynchronous request in a background thread with a JSON payload to the URL defined in the `OXITERM_APP_SERVER` environment variable.

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
| `username` | string \| null | Optional. The authenticated user's name, when the session was authenticated (SSH key / proxy-forwarded identity); `null` for anonymous/guest sessions. |
| `auth_method` | string \| null | Optional. How the session authenticated (e.g. the SSH/identity method), when known. |

### Dynamic State Patching (App Server → OxiTerm)

If the App Server returns a status code of **`200 OK`** with a JSON object, OxiTerm parses the response body as a **State Patch** and applies it to the active session state. If no state updates are needed, the response should have a status code of **`204 No Content`** (or `200 OK` with an empty body).

#### Example State Patch Response (App Server → OxiTerm)
```json
{
  "email_error": "Invalid email domain",
  "step": "3"
}
```
This patch will insert or update the keys `email_error` and `step` in the session's `StateManager`, and OxiTerm will immediately redraw the terminal UI to reflect the updated state.

> [!NOTE]
> Event dispatching by OxiTerm runs asynchronously in a spawned thread to avoid blocking the terminal event loop. The state patch is applied to the session as soon as the HTTP request completes.

---

## 2. Python Implementation

### Flask (with State Patching)
```bash
pip install flask
```

```python
from flask import Flask, request, jsonify
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
            # Return a state patch to show an error message in the UI
            return jsonify({
                "email_error": "Invalid email format"
            }), 200

    elif action == "save_record":
        print(f"Saving record for session {session_id}: {state}")

    # No state change needed
    return "", 204

if __name__ == "__main__":
    app.run(port=3000)
```

### FastAPI (asynchronous server with Pydantic and State Patching)
```bash
pip install fastapi uvicorn
```

```python
from fastapi import FastAPI, Response, status
from pydantic import BaseModel

app = FastAPI()

class OxiEvent(BaseModel):
    action: str
    state: dict[str, str]
    session_id: int

@app.post("/events")
async def handle(ev: OxiEvent, response: Response):
    match ev.action:
        case "validate_form":
            email = ev.state.get("email", "")
            if "@" not in email:
                return {"email_error": "Invalid email format"}
        case "save_record":
            await save_to_db(ev.session_id, ev.state)
            
    response.status_code = status.HTTP_204_NO_CONTENT
    return {}

async def save_to_db(session_id: int, state: dict[str, str]):
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
        return res.json({
          auth_error: "",
          logged_in: "true"
        });
      } else {
        return res.json({
          auth_error: "Invalid username or password"
        });
      }

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
  
  if (action === "check_username") {
    const username = state.username || "";
    if (username.length < 3) {
      return c.json({ username_error: "Too short!" })
    } else {
      return c.json({ username_error: "" })
    }
  }

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
use axum::{Router, Json, response::IntoResponse, http::StatusCode, routing::post};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct OxiEvent {
    action:     String,
    state:      HashMap<String, String>,
    session_id: usize,
}

async fn handle(Json(ev): Json<OxiEvent>) -> impl IntoResponse {
    match ev.action.as_str() {
        "save_config" => {
            let k = ev.state.get("config_key").cloned().unwrap_or_default();
            let v = ev.state.get("config_val").cloned().unwrap_or_default();
            println!("[Session {}] Saved: {}={}", ev.session_id, k, v);
            StatusCode::NO_CONTENT.into_response()
        }
        "validate_age" => {
            let age_str = ev.state.get("age").cloned().unwrap_or_default();
            if let Ok(age) = age_str.parse::<u32>() {
                if age < 18 {
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({ "age_error": "Must be 18 or older" }))
                    ).into_response();
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({ "age_error": "" }))
            ).into_response()
        }
        _ => StatusCode::NO_CONTENT.into_response()
    }
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

  <!-- Display auth error if set in state -->
  <text bind-state="auth_error" style="fg: #f87171; height: 1; margin-top: 1;"/>

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
    "password": "mysecretpassword",
    "auth_error": ""
  },
  "session_id": 1
}
```

If the credentials are correct, the App Server can respond with:
```json
{
  "auth_error": "",
  "logged_in": "true"
}
```
Updating the session state.

---

## 6. Concurrency and Multi-Session Patterns

When the OxiTerm server handles multiple users concurrently, the application server must properly distinguish between sessions:
* **User Identification:** Always use the `session_id` field as the unique key to identify the current connection. You can map `session_id` to a logged-in user identifier in a cache (e.g. in Redis or an in-memory HashMap).
* **State Isolation:** OxiTerm automatically isolates and maintains a separate `StateManager` state for each connected SSH/Web session. Your App Server should also maintain complete data isolation between different `session_id` values.
