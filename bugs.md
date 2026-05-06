# OxiTerm — Sprint 1 Bug Report

> Audyt: 2026-05-06 | Wersja: post-Sprint 1 | Weryfikator: `cargo check`

---

## 🔴 Błędy kompilacji (Errors)

### BUG-001 — `server::Auth::Reject` — nieprawidłowe użycie struct variant
**Plik:** `oxiterm-server/src/ssh/server.rs:25`  
**Status:** ✅ Naprawiony przez autora przed submitem (kod już zawiera `{ proceed_with_methods: None }`)  
**Wyjaśnienie:** Kompilator zwrócił `E0533` — `Auth::Reject` w russh jest struct variantem wymagającym pola `proceed_with_methods: Option<MethodSet>`. Podanie `None` jest poprawne. Commit `cargo check` który widzi błąd pochodzi z wcześniejszej wersji pliku.

> **Uwaga:** Output `cargo check` pokazuje błąd w linii 25, ale **aktualny kod w pliku** (`server.rs:25`) zawiera już `Ok(server::Auth::Reject { proceed_with_methods: None })`. Błąd jest **zamknięty** — musiał zostać naprawiony między wywołaniem `cargo check` a commitem pliku.

---

## 🟡 Ostrzeżenia kompilacji (Warnings)

### WARN-001 — Nieużywany import `parking_lot::RwLock`
**Plik:** `oxiterm-server/src/metrics.rs:2`  
**Powód:** `SessionMetrics` nie używa bezpośrednio `RwLock` — jest on używany w `session.rs`, nie w `metrics.rs`.  
**Naprawa:** Usunąć linię `use parking_lot::RwLock;` z `metrics.rs`.

### WARN-002 — Nieużywany import `Gauge`
**Plik:** `oxiterm-server/src/metrics.rs:4`  
**Powód:** `SessionMetrics` używa tylko `Counter`, nigdy `Gauge`. Import `Gauge` zbędny.  
**Naprawa:** Usunąć `Gauge` z importu prometheus.

### WARN-003 — Nieużywany import `std::io::Write`
**Plik:** `oxiterm-server/src/metrics.rs:5`  
**Powód:** Funkcja `emit_prometheus_metrics` nie przyjmuje `impl Write` — używa wewnętrznego `Vec<u8>`. Oryginalny task `S0-10` zakładał `writer: &mut impl Write`, ale implementacja poszła inną drogą (zwrot `Vec<u8>`). Obie są poprawne.  
**Naprawa:** Usunąć `use std::io::Write;`.

---

## 🔵 Problemy jakościowe / odchylenia od spec

### QUAL-001 — `auth_publickey` nie weryfikuje kluczy — zawsze zwraca `Accept`
**Plik:** `oxiterm-server/src/ssh/server.rs:21`  
**Powaga:** Krytyczna z perspektywy bezpieczeństwa  
**Opis:** Callback `auth_publickey` zawiera `TODO: implement actual key verification (S1-07)` i zawsze akceptuje każdy klucz. Oznacza to, że **każdy klient SSH może połączyć się bez weryfikacji**. Task `S1-07` jest nieukończony.  
**Wymagana naprawa:**
```rust
async fn auth_publickey(&mut self, user: &str, public_key: &key::PublicKey) 
    -> Result<server::Auth, Self::Error> 
{
    let authorized = self.auth_keys.verify(public_key);
    if authorized {
        Ok(server::Auth::Accept)
    } else {
        Ok(server::Auth::Reject { proceed_with_methods: None })
    }
}
```
`OxiServer` musi dostać pole `auth_keys: AuthorizedKeys`.

### QUAL-002 — `load_host_key` nie persystuje wygenerowanego klucza
**Plik:** `oxiterm-server/src/ssh/keys.rs:36-38`  
**Powaga:** Wysoka — operacyjna  
**Opis:** Gdy klucz hosta nie istnieje, jest generowany poprawnie, ale **nigdy nie jest zapisywany na dysk**. Przy każdym restarcie serwer prezentuje inny klucz hosta — klienci SSH dostają `REMOTE HOST IDENTIFICATION HAS CHANGED` i blokują połączenie.  
**Wymagana naprawa:** Serialize klucz do pliku PEM/OpenSSH po wygenerowaniu.

### QUAL-003 — `OxiServer::current_session` — architektoniczny problem przy multi-channel
**Plik:** `oxiterm-server/src/ssh/server.rs:11`  
**Powaga:** Średnia — przyszłościowa  
**Opis:** `current_session: Option<SessionId>` to pole na poziomie `OxiServer` (handler). W russh jeden handler = jedno połączenie SSH, więc dla normalnego przypadku (1 sesja per połączenie) jest to OK. Problem pojawia się gdy klient otworzy wiele kanałów w jednym połączeniu — `current_session` zostanie nadpisane przez ostatni `channel_open_session`. Wymagany `HashMap<ChannelId, SessionId>`.

### QUAL-004 — `RateLimiter` nie jest podłączony do SSH handlera
**Plik:** `oxiterm-server/src/ssh/mod.rs`  
**Powaga:** Średnia — bezpieczeństwo  
**Opis:** `RateLimiter` jest zdefiniowany (`ratelimit.rs`) ale **nigdy nie jest używany** w `run_server` ani w handlerze. Task `S0-15` (integracja z `channel_open_session`) jest nieukończony. Żadne połączenie nie jest rate-limitowane.

### QUAL-005 — Brak obsługi `exec_request` i `subsystem_request`
**Plik:** `oxiterm-server/src/ssh/server.rs`  
**Powaga:** Wysoka — bezpieczeństwo  
**Opis:** Tasks `S1-17` i `S1-18` nieukończone. Handler nie implementuje `exec_request` ani `subsystem_request`. Domyślne zachowanie russh przy braku implementacji to odrzucenie — **to jest poprawne zachowanie domyślne** — ale brak explicite implementacji oznacza brak logowania i możliwość zmiany zachowania przy aktualizacji russh. Należy dodać explicite callbacki zwracające błąd.

### QUAL-006 — Brak `graceful_shutdown` / `on_disconnect`
**Plik:** `oxiterm-server/src/ssh/server.rs`  
**Powaga:** Niska — operacyjna  
**Opis:** Task `S1-19` (`on_disconnect`) nieukończony. Sesje w `SessionRegistry` nigdy nie są usuwane po rozłączeniu klienta. `remove_session` istnieje w `SessionRegistry`, ale nie jest wywoływana. Przy długim działaniu serwera registry będzie rosnąć bez ograniczeń.

### QUAL-007 — `drain_sessions` — busy-loop z `sleep(100ms)`
**Plik:** `oxiterm-server/src/session.rs:50-58`  
**Powaga:** Niska — wydajność  
**Opis:** Implementacja drain to polling co 100ms. Nie blokuje to działania, ale nie jest to prawidłowy wzorzec Tokio. Powinien być użyty `tokio::sync::Notify` lub kanał do sygnalizacji zakończenia sesji.

### QUAL-008 — `SessionMetrics` nie jest podłączony do sesji
**Plik:** `oxiterm-server/src/metrics.rs`  
**Powaga:** Niska — observability  
**Opis:** `SessionMetrics` jest zdefiniowany i kompiluje się poprawnie, ale `ClientSession` nie zawiera pola `metrics: Arc<SessionMetrics>`. Endpointem `/metrics` zwraca zawsze pusty Prometheus output.

---

## 📊 Podsumowanie

| Kategoria | Liczba | Krytyczne |
|-----------|--------|-----------|
| Błędy kompilacji | 1 | 0 (naprawiony) |
| Ostrzeżenia | 3 | 0 |
| Problemy bezpieczeństwa | 2 | 1 (QUAL-001) |
| Problemy operacyjne | 4 | 1 (QUAL-002) |
| Niezaimplementowane taski | 4 | — |

### Nieukończone taski Sprint 1 (do przeniesienia)
- `S1-07` Weryfikacja kluczy w `auth_publickey`
- `S1-17` Blokada `exec_request`
- `S1-18` Blokada `subsystem_request`
- `S1-19` `on_disconnect` → czyszczenie sesji z registry
- `S0-15` Integracja `RateLimiter` z `channel_open_session`

---

## 🏅 Ocena implementacji Sprint 1

**Ogólna ocena: 6.5 / 10**

| Kryterium | Ocena | Komentarz |
|-----------|-------|-----------|
| Struktura kodu | 9/10 | Czytelna, dobry podział na moduły |
| Rust idioms | 8/10 | `parking_lot`, `Arc<RwLock>`, `async_trait` — poprawnie |
| Kompilacja | 7/10 | Warningi do posprzątania |
| Bezpieczeństwo | 3/10 | `auth_publickey` zawsze Accept — krytyczny gap |
| Operacyjność | 5/10 | Klucz nie persystuje, sesje nie czyszczone |
| Pokrycie tasków | 6/10 | ~65% tasków S1 zaimplementowanych |

**Mocne strony:** Architektura modułów jest wzorcowa. `SessionRegistry` z `parking_lot::RwLock` to poprawny wybór. Metrics server przez hyper + Prometheus to solidna podstawa. Signal handling (SIGTERM/SIGUSR1/SIGINT) — kompletny.

**Do poprawy przed Sprint 2:** QUAL-001 (auth) i QUAL-002 (klucz hosta) to blokery — bez nich serwer nie nadaje się do wdrożenia nawet testowego.
