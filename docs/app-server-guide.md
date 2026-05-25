# Integracja z Zewnętrznym Serwerem Aplikacji (App Server)

OxiTerm odpowiada za warstwę interfejsu użytkownika (THTML/TCSS), obsługę zdarzeń terminala (klawiatura/mysz) oraz zarządzanie prostym stanem sesji (`StateManager`). Jednak dla operacji wykraczających poza wbudowane akcje — takich jak walidacja formularzy, zapytania do bazy danych czy integracja z zewnętrznymi API — konieczne jest podłączenie zewnętrznego serwera aplikacji (**App Server**).

---

## 1. Protokół Komunikacji

Komunikacja między OxiTerm a zewnętrznym serwerem odbywa się za pomocą żądań HTTP POST. Przy każdym wystąpieniu zdarzenia `event-htmx`, OxiTerm wysyła asynchroniczne zapytanie z ciałem w formacie JSON na adres url zdefiniowany w zmiennej środowiskowej `OXITERM_APP_SERVER`.

### Schemat Payloadu JSON (OxiTerm → App Server)

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

### Opis Pól

| Pole | Typ | Opis |
|---|---|---|
| `action` | string | Surowa wartość atrybutu `event-htmx` (np. `"login"`, `"save_record"`), która wyzwoliła to zdarzenie. |
| `state` | object | Słownik (mapa klucz-wartość) reprezentujący bieżący stan sesji. Wszystkie wartości stanu są przesyłane w formie ciągów znaków (String). |
| `session_id` | integer | Unikalny identyfikator przypisany do danej sesji SSH lub WebSocket. Służy do rozróżniania poszczególnych połączonych użytkowników. |

> [!WARNING]
> Wysyłanie zdarzeń przez OxiTerm odbywa się w trybie **fire-and-forget** (wyślij i zapomnij). Serwer OxiTerm nie czeka na odpowiedź zwrotną (HTTP response) z serwera aplikacji przed ponownym wyrenderowaniem ekranu. Odpowiedź z serwera aplikacji powinna mieć kod statusu `204 No Content` lub `200 OK` z pustym body.

---

## 2. Implementacja w Pythonie

### Flask (prosty przykład)
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
            print(f"Sesja {session_id}: Niepoprawny format adresu email")

    elif action == "save_record":
        print(f"Zapisywanie rekordu dla sesji {session_id}: {state}")

    return "", 204

if __name__ == "__main__":
    app.run(port=3000)
```

### FastAPI (asynchroniczny serwer z Pydantic)
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
    # Logika walidacji...
    pass

async def save_to_db(session_id: int, state: dict[str, str]):
    # Zapis do bazy...
    pass

# Uruchomienie: uvicorn server:app --port 3000
```

---

## 3. Implementacja w Node.js (JavaScript / TypeScript)

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
        console.log(`Sesja ${session_id}: autoryzacja powiodła się`);
      }
      break;

    case "fetch_data":
      fetchFromDB(state).then(data => {
        console.log(`Sesja ${session_id}: pobrano dane`, data);
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
  console.log(`[Sesja ${session_id}] Zdarzenie: ${action}`, state)
  return c.body(null, 204)
})

export default { port: 3000, fetch: app.fetch }
```

---

## 4. Implementacja w Rust (Axum)

Uruchomienie serwera aplikacji w języku Rust pozwala na współdzielenie typów struktur zdarzeń (np. poprzez wspólny crate w workspace).

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
            println!("[Sesja {}] Zapisano: {}={}", ev.session_id, k, v);
        }
        "run_job" => {
            tokio::spawn(async move {
                // Wykonanie długotrwałego zadania w tle...
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

## 5. Kompletny Przepływ (Formularz logowania)

Oto jak zbudować interfejs THTML, aby poprawnie przekazywał dane wejściowe do Twojego serwera aplikacji:

```html
<style>
  .field { border-style: single; border-color: #334155;
           height: 1; padding-left: 1; margin-top: 1; }
  .btn   { border-style: single; border-color: #4ade80;
           padding: 1; height: 3; align-items: center; }
</style>

<box style="flex-direction: column; width: 60; height: 24; bg: #0f172a;">
  <text style="fg: #38bdf8; height: 1; margin-bottom: 2;">Panel Logowania</text>

  <text style="fg: #94a3b8; height: 1;">Nazwa użytkownika:</text>
  <input bind-value="username" class="field" placeholder="Wpisz nazwę..."/>

  <text style="fg: #94a3b8; height: 1; margin-top: 1;">Hasło:</text>
  <input bind-value="password" class="field" placeholder="••••••"/>

  <!-- Kliknięcie wyzwoli akcję "login" -->
  <box class="btn" style="margin-top: 2;" event-htmx="login">
    <text style="fg: #4ade80; height: 1;">Zaloguj się</text>
  </box>
</box>
```

Gdy użytkownik uzupełni pola i kliknie przycisk "Zaloguj się", OxiTerm wyśle do App Servera następujące żądanie POST:
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

## 6. Współbieżność i Wzorce Wielosesyjne

Gdy serwer OxiTerm obsługuje wielu użytkowników jednocześnie, serwer aplikacji musi poprawnie rozróżniać sesje:
* **Identyfikacja użytkownika:** Zawsze używaj pola `session_id` jako unikalnego klucza do identyfikacji bieżącego połączenia. Możesz mapować `session_id` na identyfikator zalogowanego użytkownika w pamięci podręcznej (np. w bazie Redis lub w HashMapie w pamięci procesu).
* **Izolacja stanu:** OxiTerm automatycznie izoluje i przechowuje osobny stan `StateManager` dla każdej podłączonej sesji SSH/Web. Twój App Server również powinien zachować pełną izolację danych pomiędzy różnymi wartościami `session_id`.
