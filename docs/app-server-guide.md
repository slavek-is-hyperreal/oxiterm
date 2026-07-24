# Integration with an External Application Server (App Server)

OxiTerm is responsible for the user interface layer (THTML/TCSS), handling terminal events (keyboard/mouse), and managing basic session state (`StateManager`). However, for operations that go beyond built-in actions — such as form validation, database queries, or external API integration — you must connect an external application server (**App Server**).

---

## 1. Authentication

Both channels between OxiTerm and your App Server are protected by a shared secret token.

### Setting the token

Set `OXITERM_APP_TOKEN` on the OxiTerm server:
```bash
export OXITERM_APP_TOKEN="<your-secret-token>"
```

Set the **same** token on your App Server and verify it on every incoming request.

### OxiTerm → App Server (Bearer header)

OxiTerm attaches the token to every outgoing `POST /events` request:
```
Authorization: Bearer <token>
```

Your App Server **must** verify this header with a constant-time comparison (e.g. `secrets.compare_digest` in Python, `crypto.timingSafeEqual` in Node). Reject requests with a wrong or missing token with `401 Unauthorized`.

### Enabling / disabling the push endpoint

If `OXITERM_APP_TOKEN` is **unset or empty**, OxiTerm disables the inbound push endpoint entirely — all `POST /sessions/{id}/patch` requests receive `404 Not Found`. Setting a non-empty token enables it. This is a **circuit breaker**, not optional hardening.

> [!IMPORTANT]
> All five code examples below include the `Authorization: Bearer` check. An App Server that skips this check allows any caller on the network to inject arbitrary state into any active user session.

---

## 2. Communication Protocol

### OxiTerm → App Server (`POST /events`)

On every `event-htmx` event, OxiTerm sends an asynchronous HTTP POST to the URL in `OXITERM_APP_SERVER`:

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

| Field | Type | Description |
|---|---|---|
| `action` | string | The raw value of the `event-htmx` attribute (e.g. `"login"`, `"save_record"`) that triggered the event. |
| `state` | object | A key-value dictionary representing the current session state. All state values are sent as strings. |
| `session_id` | integer | The unique identifier assigned to the given SSH or WebSocket session. Used to distinguish between individual connected users. |
| `username` | string \| null | Optional. The authenticated user's name, when the session was authenticated (SSH key / proxy-forwarded identity); `null` for anonymous/guest sessions. |
| `auth_method` | string \| null | Optional. How the session authenticated (e.g. the SSH/identity method), when known. |

#### State Patch Response (App Server → OxiTerm)

If the App Server returns `200 OK` with a JSON object, OxiTerm applies it as a **state patch** to the session. For no state change, return `204 No Content` (or `200 OK` with an empty body).

```json
{
  "email_error": "Invalid email domain",
  "step": "3"
}
```

> [!NOTE]
> Event dispatching runs asynchronously in a spawned thread to avoid blocking the terminal event loop. The state patch is applied as soon as the HTTP request completes.

---

### App Server → OxiTerm (`POST /sessions/{id}/patch`)

Your App Server can push state patches to OxiTerm at any time — without waiting for a user event:

```
POST /sessions/{session_id}/patch
Authorization: Bearer <token>
Content-Type: application/json

{"now_playing": "Dark Side of the Moon", "progress": "42"}
```

| Response code | Meaning |
|---|---|
| `200` | Patch applied to the session |
| `401` | Token missing or wrong |
| `404` | Endpoint disabled (empty token) **or** session does not exist |
| `400` | `{id}` is not a valid integer |

> [!NOTE]
> The mapping from `session_id` to a user identity belongs to your App Server. **Never send that mapping back** to OxiTerm in a state patch — it would become visible to the client.

---

## 3. Python Implementation

### Flask (with Bearer auth + State Patching)
```bash
pip install flask
```

```python
import hmac, os
from flask import Flask, request, jsonify, abort

app = Flask(__name__)
APP_TOKEN = os.environ["OXITERM_APP_TOKEN"]

def require_bearer():
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer "):
        abort(401)
    token = auth[len("Bearer "):]
    if not hmac.compare_digest(token, APP_TOKEN):
        abort(401)

@app.route("/events", methods=["POST"])
def handle_event():
    require_bearer()
    payload    = request.json
    action     = payload["action"]
    state      = payload["state"]
    session_id = payload["session_id"]

    if action == "validate_form":
        email = state.get("email", "")
        if "@" not in email:
            return jsonify({"email_error": "Invalid email format"}), 200

    elif action == "save_record":
        print(f"Saving record for session {session_id}: {state}")

    return "", 204

if __name__ == "__main__":
    app.run(port=3000)
```

### FastAPI (asynchronous, with Bearer auth + State Patching)
```bash
pip install fastapi uvicorn
```

```python
import hmac, os
from fastapi import FastAPI, Header, HTTPException, Response, status
from pydantic import BaseModel

app = FastAPI()
APP_TOKEN = os.environ["OXITERM_APP_TOKEN"]

def require_bearer(authorization: str = Header(...)):
    if not authorization.startswith("Bearer "):
        raise HTTPException(status_code=401)
    token = authorization[len("Bearer "):]
    if not hmac.compare_digest(token, APP_TOKEN):
        raise HTTPException(status_code=401)

class OxiEvent(BaseModel):
    action: str
    state: dict[str, str]
    session_id: int

@app.post("/events")
async def handle(ev: OxiEvent, response: Response,
                 authorization: str = Header(...)):
    require_bearer(authorization)
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

## 4. Node.js Implementation (JavaScript / TypeScript)

### Express (with Bearer auth)
```bash
npm install express
```

```javascript
const express = require("express");
const crypto  = require("crypto");
const app = express();
app.use(express.json());

const APP_TOKEN = process.env.OXITERM_APP_TOKEN;

function requireBearer(req, res, next) {
  const auth = req.headers["authorization"] || "";
  if (!auth.startsWith("Bearer ")) return res.status(401).end();
  const token = auth.slice("Bearer ".length);
  // constant-time comparison
  try {
    const a = Buffer.from(token);
    const b = Buffer.from(APP_TOKEN);
    if (a.length !== b.length || !crypto.timingSafeEqual(a, b))
      return res.status(401).end();
  } catch { return res.status(401).end(); }
  next();
}

app.post("/events", requireBearer, (req, res) => {
  const { action, state, session_id } = req.body;

  switch (action) {
    case "login":
      if (state.username === "admin" && state.password === "secret") {
        return res.json({ auth_error: "", logged_in: "true" });
      } else {
        return res.json({ auth_error: "Invalid username or password" });
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

### Hono (Bun / Edge, with Bearer auth)
```javascript
import { Hono } from "hono"
import { timingSafeEqual } from "node:crypto"

const app = new Hono()
const APP_TOKEN = process.env.OXITERM_APP_TOKEN

function checkBearer(c) {
  const auth = c.req.header("Authorization") || ""
  if (!auth.startsWith("Bearer ")) return false
  const token = auth.slice("Bearer ".length)
  try {
    const a = Buffer.from(token)
    const b = Buffer.from(APP_TOKEN)
    return a.length === b.length && timingSafeEqual(a, b)
  } catch { return false }
}

app.post("/events", async c => {
  if (!checkBearer(c)) return c.body(null, 401)
  const { action, state, session_id } = await c.req.json()

  if (action === "check_username") {
    const username = state.username || ""
    if (username.length < 3) return c.json({ username_error: "Too short!" })
    return c.json({ username_error: "" })
  }

  return c.body(null, 204)
})

export default { port: 3000, fetch: app.fetch }
```

---

## 5. Rust Implementation (Axum, with Bearer auth)

```toml
[dependencies]
axum       = "0.7"
tokio      = { version = "1", features = ["full"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
```

```rust
use axum::{
    Router, Json, extract::TypedHeader,
    headers::{Authorization, authorization::Bearer},
    response::IntoResponse, http::StatusCode, routing::post,
};
use serde::Deserialize;
use std::{collections::HashMap, env};

#[derive(Deserialize)]
struct OxiEvent {
    action:     String,
    state:      HashMap<String, String>,
    session_id: usize,
}

fn token_ok(bearer: &str) -> bool {
    let expected = env::var("OXITERM_APP_TOKEN").unwrap_or_default();
    // Constant-time comparison
    if bearer.len() != expected.len() { return false; }
    bearer.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

async fn handle(
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(ev): Json<OxiEvent>,
) -> impl IntoResponse {
    if !token_ok(auth.token()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
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
                    return (StatusCode::OK,
                        Json(serde_json::json!({ "age_error": "Must be 18 or older" }))
                    ).into_response();
                }
            }
            (StatusCode::OK, Json(serde_json::json!({ "age_error": "" }))).into_response()
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

## 6. Complete Flow (Login Form Example)

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
  <input type="password" bind-value="password" class="field" placeholder="••••••"/>

  <!-- Display auth error if set in state -->
  <text bind-state="auth_error" style="fg: #f87171; height: 1; margin-top: 1;"/>

  <!-- Clicking will trigger the "login" action -->
  <box class="btn" style="margin-top: 2;" event-htmx="login">
    <text style="fg: #4ade80; height: 1;">Log In</text>
  </box>
</box>
```

When the user fills in the fields and clicks "Log In", OxiTerm sends:
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

If the credentials are correct, the App Server responds with:
```json
{
  "auth_error": "",
  "logged_in": "true"
}
```

---

## 7. Concurrency and Multi-Session Patterns

When OxiTerm handles multiple users concurrently, your App Server must properly distinguish between sessions:

* **User Identification:** Use `session_id` as the unique key for the current connection. Map it to a logged-in user identifier in a cache (e.g. Redis or an in-memory dictionary).
* **State Isolation:** OxiTerm automatically isolates a separate `StateManager` per session. Your App Server must also maintain complete data isolation between different `session_id` values.
* **Identity Mapping Security:** The mapping from `session_id` to a user identity belongs to your App Server. **Do not send it back to OxiTerm in a state patch** — doing so exposes one user's identity to another user's client if sessions are ever confused.
