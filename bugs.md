# OxiTerm — Kompleksowy Raport Bugów (Post All-Sprints Audit)

> **Audyt:** 2026-05-06 | Wersja: post-Sprint 5 (wszystkie sprinty "gotowe")  
> **Metoda:** Manualna inspekcja wszystkich plików `.rs` w projekcie  
> **Ocena ogólna: 5.5 / 10**

---

## 🔴 BŁĘDY KRYTYCZNE — Blokery produkcyjne

### BUG-C01 — `auth_password` AKCEPTUJE KAŻDE HASŁO
**Plik:** `oxiterm-server/src/ssh/server.rs:32-35`  
**Powaga:** 🚨 KRYTYCZNA — luka bezpieczeństwa  
**Opis:** `auth_password` zawiera komunikat `"Accepted password auth for test user"` i zwraca `Auth::Accept`. Specyfikacja wymaga **odrzucenia** każdego logowania hasłem. Serwer jest aktualnie dostępny dla każdego kto zna adres IP i ma dowolne hasło.
```rust
// OBECNY KOD — NIE MOŻE TRAFIĆ NA PRODUKCJĘ:
async fn auth_password(&mut self, user: &str, _password: &str) -> Result<server::Auth, Self::Error> {
    info!("Accepted password auth for test user: {user}");
    Ok(server::Auth::Accept)  // ← KATASTROFA
}

// POPRAWNA IMPLEMENTACJA:
async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<server::Auth, Self::Error> {
    Ok(server::Auth::Reject { proceed_with_methods: None })
}
```

### BUG-C02 — `load_host_key` NIGDY NIE ZAPISUJE KLUCZA NA DYSK
**Plik:** `oxiterm-server/src/ssh/keys.rs:33-42`  
**Powaga:** 🚨 KRYTYCZNA — operacyjna  
**Opis:** Gdy klucz hosta nie istnieje, jest generowany i **natychmiast zwracany bez zapisu**. Przy każdym restarcie serwera klienci SSH dostają `REMOTE HOST IDENTIFICATION HAS CHANGED` i zrywają połączenie. Komentarz `// Save key (simplified)` jest pustą obietnicą.

### BUG-C03 — `EventLoop::run()` BLOKUJE WĄTEK TOKIO PRZEZ `std::thread::sleep`
**Plik:** `oxiterm-server/src/session.rs:201-205`  
**Powaga:** 🚨 KRYTYCZNA — wydajność runtime  
**Opis:** Linia 205: `std::thread::sleep(std::time::Duration::from_millis(200))`. `EventLoop::run()` jest wywoływany z `std::thread::spawn` (linia `server.rs:81`), więc blokowanie jest akceptowalne. **ALE** komentarz sugeruje "zabezpieczenie race-condition w SSH" — to jest patchwork na głębszy problem. Właściwe rozwiązanie to oczekiwanie na potwierdzenie `channel_success` lub `session_start` przez kanał sygnałowy, nie `sleep(200ms)`.

### BUG-C04 — `SyncedEmitter::emit_frame` PODWAJA BSU/ESU
**Plik:** `oxiterm-server/src/render/emitter.rs:11-33`  
**Powaga:** 🔴 WYSOKA — defekt protokołu  
**Opis:** `emit_frame` wysyła `\x1b[?2026h` (BSU) na liniach 19. Następnie wywołuje `DiffEngine::encode_ansi()`, która **SAMA WEWNĘTRZNIE** dodaje BSU na linii 79 i ESU na linii 122 (`diff.rs`). Następnie `emit_frame` wysyła ESU ponownie (linia 28). Klient SSH dostaje sekwencję:
```
BSU → BSU → [dane] → ESU → ESU
```
Podwójne BSU/ESU jest niezdefiniowanym zachowaniem w specyfikacji terminali. Część emulatorów zignoruje to, część może się zawiesić. Jedno z dwóch miejsc musi być usunięte.

---

## 🟠 BŁĘDY WYSOKIEJ WAGI — Poważne defekty logiczne

### BUG-H01 — `PredictiveEcho::confirm` UŻYWA WYSZUKIWANIA PO WARTOŚCI ZNAKU
**Plik:** `oxiterm-server/src/session.rs:44-50`  
**Powaga:** 🟠 WYSOKA — błąd logiczny  
**Opis:** `confirm(ch)` usuwa **pierwsze wystąpienie znaku** z bufora, nie **pierwszy znak z kolejki FIFO**. Przy wpisaniu "aaaa" i potwierdzeniu 'a' usunie zawsze indeks 0, ale przy "abc" + potwierdzeniu 'c' usunie 'c' nawet jeśli to nie jest następny w kolejności do potwierdzenia przez serwer. Właściwe echo predykcyjne wymaga struktury FIFO (kolejki), nie wyszukiwania po wartości.
```rust
// OBECNY (BŁĘDNY):
pub fn confirm(&mut self, ch: char) {
    if let Some(pos) = self.buffer.chars().position(|c| c == ch) { // szuka wartości!
        // ...
    }
}
// POPRAWNE: pop z przodu kolejki i porównanie
```

### BUG-H02 — `PredictiveEcho` RYSUJE OVERLAY W STAŁYM MIEJSCU `(0, 0)`
**Plik:** `oxiterm-server/src/session.rs:270-281`  
**Powaga:** 🟠 WYSOKA — błąd funkcjonalny  
**Opis:** Linia 271: `let cursor_y = 0; // Top-left for now`. Nakładka predykcyjna jest zawsze rysowana od lewego górnego rogu ekranu, ignorując pozycję aktywnego węzła `<input>`. Dla każdego UI który nie ma inputa w `(0,0)` — a żaden sensowny UI nie ma — efekt jest wizualnym artefaktem. Komentarz "Simple overlay logic" maskuje brak prawdziwej implementacji.

### BUG-H03 — `window_change_request` WYSYŁA PUSTY `Vec::new()` JAKO "SYGNAŁ"
**Plik:** `oxiterm-server/src/ssh/server.rs:128`  
**Powaga:** 🟠 WYSOKA — błąd architektoniczny  
**Opis:** `session.raw_input_tx.send(Vec::new())` — pusty wektor jest wysyłany do `ReactorThread` żeby "wybudzić" EventLoop po resize. Problem: `ReactorThread` jest zaprojektowany do przetwarzania surowych bajtów wejścia SSH. Pusty slice prawdopodobnie zostanie zignorowany lub spowoduje nieoczekiwane zachowanie parsera. Właściwe rozwiązanie: osobny kanał sygnałowy `tokio::sync::Notify` lub enum `ReactorCommand::Resize`.

### BUG-H04 — `diff.rs` KOMENTUJE `cur_x = Some(x)` — BRAKUJE AKTUALIZACJI STANU
**Plik:** `oxiterm-renderer/src/render/diff.rs:37`  
**Powaga:** 🟠 WYSOKA — błąd logiczny diff engine  
**Opis:** Linia 37: `// cur_x = Some(x); // Wygłuszony warning - nie używamy tej wartości dalej`. To jest **zamierzony błąd**: `cur_x` jest aktualizowane po zapisaniu znaku (linia 63: `cur_x = Some(x + 1)`), ale nie po emisji `MoveCursor`. Znaczy to, że gdy dwie różniące się komórki są na tym samym wierszu pod rząd, kod sprawdza `cur_x != Some(x)` — gdzie `cur_x` jest wartością **sprzed** poprzedniego `MoveCursor`, nie po nim. Może to powodować zbędne duplikaty `MoveCursor` w skrajnych przypadkach (nie powoduje korupcji, ale wpływa na rozmiar ramki).

### BUG-H05 — `LayoutEngine` NIGDY NIE CZYŚCI `dirty_nodes` PO OBLICZENIU
**Plik:** `oxiterm-renderer/src/layout/engine.rs:29-81`  
**Powaga:** 🟠 WYSOKA — wyciek logiczny  
**Opis:** `compute()` odczytuje `doc.dirty_nodes` (linia 34) i synchronizuje zmiany, ale **nie wywołuje `doc.clear_dirty()`**. Każde kolejne wywołanie `compute()` będzie ponownie przetwarzać te same węzły jako "brudne", niwecząc całą optymalizację incremental update. `clear_dirty()` istnieje w `document.rs:58`, ale nikt jej nie wywołuje.

### BUG-H06 — `RateLimiter` NIE JEST PODŁĄCZONY DO SSH HANDLERA
**Plik:** `oxiterm-server/src/ratelimit.rs` / `ssh/mod.rs`  
**Powaga:** 🟠 WYSOKA — brak ochrony  
**Opis:** `RateLimiter` jest zdefiniowany i poprawnie działa, ale nigdzie nie jest tworzony ani używany w `run_server` ani w `OxiServer`. Żadne połączenie SSH nie jest rate-limitowane. Task S0-15 jest zaznaczony jako `[x]` w `task.md` — to nieprawda.

---

## 🟡 BŁĘDY ŚREDNIEJ WAGI — Defekty funkcjonalne

### BUG-M01 — `ResizeDebouncer::poll` SPRAWDZA `elapsed()` OD `last_update`, NIE OD `push`
**Plik:** `oxiterm-server/src/session.rs:68-77`  
**Powaga:** 🟡 ŚREDNIA — błąd logiczny debouncer  
**Opis:** Debouncer ma sprawdzać czy minęło >100ms od **ostatniego sygnału resize** (`push`). Aktualnie sprawdza czas od `last_update` (który jest aktualizowany przy `poll`). Jeśli resize zostanie wywołany 1ms przed `poll`, zostanie od razu zwrócony, pomijając intencję debounce. Brakuje `pushed_at: Instant` aktualizowanego w `push()`.

### BUG-M02 — `NodeArena::compact` NIE AKTUALIZUJE `doc.root` W DOKUMENCIE
**Plik:** `oxiterm-renderer/src/arena.rs:56-83`  
**Powaga:** 🟡 ŚREDNIA — błąd korektności  
**Opis:** `compact()` zwraca `HashMap<NodeId, NodeId>` (RemapTable) i aktualizuje `children` wewnątrz węzłów — ale **nie aktualizuje** `doc.root`, który może teraz wskazywać na nieistniejący lub inny węzeł. Wywołujący musi ręcznie zaktualizować `root`, co jest ukrytą pułapką. Powinna to robić metoda na `THTMLDocument`, nie na `NodeArena`.

### BUG-M03 — `parse_thtml` IGNORUJE `sanitize_style_raw` PRZY BŁĘDZIE PARSOWANIA
**Plik:** `oxiterm-renderer/src/parser/thtml.rs:213-218`  
**Powaga:** 🟡 ŚREDNIA — bezpieczeństwo  
**Opis:** `sanitize_style_raw` jest wywoływany w `parse_attr_kv` (poprawnie), ale tylko dla klucza `"style"`. Atrybut `"event-htmx"` (linia 197) nie jest sanityzowany. Wartość HTMX może zawierać arbitralne znaki — przy błędzie parsowania lub przyszłym użyciu ta wartość może stanowić wektor ataku.

### BUG-M04 — `BoundedFrameChannel::Drop` SPRAWDZA `strong_count <= 2` — HEURYSTYKA KRUCHA
**Plik:** `oxiterm-server/src/backpressure.rs:96-103`  
**Powaga:** 🟡 ŚREDNIA — race condition  
**Opis:** `if Arc::strong_count(&self.inner) <= 2` to próba wykrycia "czy receiver już dropped". Problem: `strong_count` może być `2` z powodów innych niż dropped receiver (np. tymczasowe klonowanie). Może to doprowadzić do przedwczesnego zamknięcia kanału lub braku zamknięcia. Właściwe rozwiązanie: flaga `AtomicBool sender_dropped` lub użycie `Arc::try_unwrap`.

### BUG-M05 — `LayoutEngine::compute` UŻYWA `unwrap()` NA WYNIKU TAFFY
**Plik:** `oxiterm-renderer/src/layout/engine.rs:71`  
**Powaga:** 🟡 ŚREDNIA — panic risk  
**Opis:** Linia 71: `let layout = self.taffy.layout(taffy_id).unwrap();`. Jeśli Taffy nie posiada layoutu dla danego węzła (np. po błędzie `compute_layout`), wywołanie `unwrap()` powoduje panic całego wątku EventLoop, kończąc sesję użytkownika bez graceful error handling. Powinno być `?` lub `map_err`.

### BUG-M06 — `KittyImageManager::transmit_image` PRZESYŁA SUROWE BAJTY BASE64 ZAMIAST STRING
**Plik:** `oxiterm-renderer/src/render/kitty.rs:9,23`  
**Powaga:** 🟡 ŚREDNIA — błąd protokołu  
**Opis:** `b64.as_bytes().chunks(4096)` — chunk'owanie po bajtach UTF-8 stringa Base64 jest poprawne tylko gdy Base64 jest pure ASCII (co jest gwarantowane przez standard). **Ale** linia 23: `output.extend_from_slice(chunk)` dokłada bajty bezpośrednio po headerze tekstowym bez separatora. Kitty Protocol wymaga że payload Base64 jest częścią sekwencji APC (`\x1b_...\x1b\\`), a header i payload muszą być oddzielone średnikiem. Aktualnie header kończy się na `m=N;` i payload jest doklejony poprawnie, ale dla chunku `i > 0` header to tylko `\x1b_Gm=N;` bez `a=` (action). To może powodować odrzucenie przez terminal.

### BUG-M07 — `insert_vtm_modifier` UŻYWA SURROGATES (U+D0000) — NIEPRAWIDŁOWY UNICODE
**Plik:** `oxiterm-renderer/src/render/unicode.rs:39`  
**Powaga:** 🟡 ŚREDNIA — błąd Unicode  
**Opis:** `U+D0000` leży w **Supplementary Private Use Area-B** (`U+100000–U+10FFFF`), nie w surogatach. To jest poprawny PUA. **ALE** `std::char::from_u32(0xD_0000)` to `0xD0000` hex = `851968` dec — sprawdź: `U+D0000` jest w Supplementary PUA i jest poprawny. Jednak specyfikacja VTM (dokumentacja `Amendment.md`) podaje zakres `U+D0000–U+D08F6`. `cluster_width` jest `u8` (0-255), więc `0xD0000 + cluster_width` może wyjść poza `U+D08F6` dla wartości >142. Brak sprawdzenia zakresu.

---

## 🔵 PROBLEMY JAKOŚCIOWE / ODCHYLENIA OD SPEC

### QUAL-01 — `EventBus::dispatch` NIE JEST WYWOŁYWANE Z EVENTLOOP
**Plik:** `oxiterm-server/src/session.rs:223-254`  
**Opis:** `EventLoop::run()` obsługuje `InputEvent::MouseEvent` logując tylko `info!("Mouse: {:?}", m)` (linia 236). Nie wywołuje `EventBus::dispatch`. Cała architektura zdarzeń HTMX (Sprint 4) jest zdefiniowana, ale nieużywana. Kliknięcia myszy nie działają.

### QUAL-02 — `FrameRateLimiter` ZDEFINIOWANY, NIE UŻYWANY W EVENTLOOP
**Plik:** `oxiterm-renderer/src/render/limiter.rs`  
**Opis:** `FrameRateLimiter` ma poprawną logikę, ale `EventLoop::run()` renderuje przy każdym zdarzeniu (`needs_render = true`) bez żadnego throttlingu FPS. Przy szybkim wpisywaniu można wygenerować 1000 ramek/s dla jednej sesji.

### QUAL-03 — `SixelCodec::encode_image` TO STUB — ZAWSZE ZWRACA CZARNY PROSTOKĄT
**Plik:** `oxiterm-renderer/src/render/sixel.rs:5-19`  
**Opis:** Implementacja zwraca statyczny string `"#0;2;0;0;0"` (kolor czarny) + `"!100~"` (100 pikseli), ignorując `rgba_data` i rzeczywiste wymiary. Każdy obraz wyświetlony przez Sixel będzie czarnym prostokątem 100 pikseli. `_rgba_data` jest prefixem z `_` — jawne przyznanie że parametr jest ignorowany.

### QUAL-04 — `auth_password` KOMENTARZ "test user" SUGERUJE ŚWIADOME POZOSTAWIENIE
**Plik:** `oxiterm-server/src/ssh/server.rs:33`  
**Opis:** Log `"Accepted password auth for test user: {user}"` sugeruje że ktoś świadomie zostawił to dla developmentu. Jednak w `task.md` S1-10 jest zaznaczony jako `[x]`. Albo task.md kłamie, albo ktoś "zapomniał" wyłączyć tryb testowy.

### QUAL-05 — `clear()` W `CellBuffer` UŻYWA PĘTLI ZAMIAST `fill()`
**Plik:** `oxiterm-renderer/src/render/buffer.rs:56-60`  
**Opis:** Wydajność: `for cell in &mut self.cells { *cell = Cell::default(); }` — Rust nie może wektoryzować tego automatycznie ponieważ `Cell::default()` nie jest trywialnym wzorcem bitowym. Lepiej: `self.cells.fill(Cell::default())` lub (dla zero-fill) `unsafe { std::ptr::write_bytes(...) }`.

### QUAL-06 — BRAK TESTÓW JEDNOSTKOWYCH W CAŁYM PROJEKCIE
**Opis:** Zero plików z `#[cfg(test)]`. Wszystkie taski testowe w `task.md` (S1-22..24, S2-23..26, S3-36..38, S4-32..35, S5-33..36) są zaznaczone albo jako `[ ]` albo `[x]` bez żadnego kodu testowego. `cargo test` zwróci `running 0 tests`.

### QUAL-07 — `RenderMode` ZDEFINIOWANY ALE `select_render_mode` BRAK
**Plik:** `oxiterm-renderer/src/render/limiter.rs`  
**Opis:** `FrameRateLimiter` istnieje, ale brak `RenderMode` enum i `select_render_mode()`. Task S5-06 jest `[/]` (partial), S5-07 jest `[ ]`.

### QUAL-08 — `negotiate_capabilities` I `negotiator.rs` NIEZWERYFIKOWANE
**Plik:** `oxiterm-server/src/ssh/server.rs:70`  
**Opis:** `crate::ssh::negotiator::negotiate_capabilities(channel, session)?` — nie ma dostępu do pliku `negotiator.rs` w obecnej strukturze katalogów z listy plików. Jeśli ten plik nie istnieje lub ma błędy, `cargo build` się nie skompiluje.

---

## 📊 Podsumowanie bugów

| Kategoria | Liczba | Blokujące prod. |
|-----------|--------|-----------------|
| 🔴 Krytyczne | 4 | 4 |
| 🟠 Wysokiej wagi | 6 | 2 |
| 🟡 Średniej wagi | 7 | 0 |
| 🔵 Jakościowe | 8 | 0 |
| **SUMA** | **25** | **6** |

---

## 🏅 Ocena całościowa: **5.5 / 10**

| Kryterium | Ocena | Komentarz |
|-----------|-------|-----------|
| Architektura modułów | 8/10 | Dobry podział, RRT, kanały — widać kierunek |
| Rust idioms | 7/10 | `parking_lot`, `Arc`, `OnceLock` — poprawnie |
| **Bezpieczeństwo** | **1/10** | `auth_password` akceptuje wszystkich. JEDEN BŁĄD ZABIJA CAŁY SERWER |
| Kompletność implementacji | 5/10 | Wiele modułów to szkielety lub stuby |
| Testy | 0/10 | Zero testów. Dosłownie zero. |
| Correctness (logika) | 5/10 | PredictiveEcho, DiffEngine, Debouncer — błędy logiczne |
| Operacyjność | 4/10 | Klucz nie persystuje, rate limit podłączony do nicości |

### Top 3 rzeczy do naprawienia NATYCHMIAST (przed jakimkolwiek wdrożeniem):
1. **BUG-C01** — `auth_password` → `Auth::Reject`. 3 linie kodu. Brak excusy.
2. **BUG-C04** — BSU/ESU podwójne. Usunąć z `DiffEngine::encode_ansi` (zostają tylko w `SyncedEmitter`).
3. **BUG-H05** — `clear_dirty()` po `compute()` w `LayoutEngine`. 1 linia kodu, fundamentalna dla wydajności.
