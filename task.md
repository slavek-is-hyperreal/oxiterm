# OxiTerm — Task Backlog

> Granularność: 1 funkcja / 1 obiekt = 1 task  
> Status: `[ ]` todo · `[/]` in progress · `[x]` done

> **Zmiany po audycie (2026-05-06):** poprawiony `S3-20` (flat Vec), dodano `S2-09b` defragmentacja Areny, `S2-02b` sanityzacja `style_raw`, Sprint 0 (infrastruktura) i Sprint 7 (operacyjność).

---

## Sprint 0 — Infrastruktura i Szkielet Projektu

> Cel: Środowisko deweloperskie, CI/CD, konfiguracja runtime — wykonać **przed** Sprint 1.

### 0.1 Workspace Cargo
- [x] `S0-01` Plik `Cargo.toml` workspace z crate'ami: `oxiterm-server`, `oxiterm-proto`, `oxiterm-renderer`, `oxiterm-a11y`
- [x] `S0-02` Plik `.cargo/config.toml` — target linker, clippy lints (`#![deny(unsafe_op_in_unsafe_fn)]`)
- [x] `S0-03` Plik `rust-toolchain.toml` — przypięcie wersji rustc dla reproducibility

### 0.2 Konfiguracja runtime
- [x] `S0-04` Struct `OxiTermConfig` — wszystkie parametry serwera (port, bind addr, key path, max sessions, fps limit)
- [x] `S0-05` Funkcja `OxiTermConfig::from_file(path: &Path) -> Result<OxiTermConfig>` — ładowanie z TOML
- [x] `S0-06` Funkcja `OxiTermConfig::from_env() -> Result<OxiTermConfig>` — override przez zmienne środowiskowe
- [x] `S0-07` Funkcja `OxiTermConfig::validate(&self) -> Result<()>` — walidacja spójności config

### 0.3 Logowanie i telemetria
- [x] `S0-08` Inicjalizacja `tracing_subscriber` z filtrowaniem po `RUST_LOG`
- [x] `S0-09` Struct `SessionMetrics` — `connected_at`, `bytes_sent`, `bytes_recv`, `frame_count`, `drop_count`
- [x] `S0-10` Funkcja `emit_prometheus_metrics(metrics: &SessionMetrics, writer: &mut impl Write)` — eksport w formacie Prometheus text
- [x] `S0-11` Endpoint `GET /metrics` (HTTP mini-server na osobnym porcie) — scraping przez Prometheus

### 0.4 Rate limiting i ochrona
- [x] `S0-12` Struct `RateLimiter` — sliding window counter per IP (`HashMap<IpAddr, WindowCounter>`)
- [x] `S0-13` Metoda `RateLimiter::check_and_record(ip: IpAddr) -> RateResult` — sprawdzenie limitu połączeń/min
- [x] `S0-14` Enum `RateResult` — `Allow`, `Throttle(Duration)`, `Deny`
- [x] `S0-15` Integracja `RateLimiter` w handlerze `channel_open_session` (Sprint 1)

### 0.5 CI/CD
- [x] `S0-16` Plik `.github/workflows/ci.yml` — build + clippy + test na każdy PR
- [x] `S0-17` Job `cargo audit` — skanowanie CVE w zależnościach
- [x] `S0-18` Job cross-compilation `x86_64-unknown-linux-musl` — statyczny binarny dla deploymentu
- [x] `S0-19` Job `cargo tarpaulin` — raport pokrycia kodu testami

### 0.6 Graceful restart
- [x] `S0-20` Handler sygnału `SIGUSR1` — inicjacja graceful drain (brak nowych połączeń, dokończenie aktywnych)
- [x] `S0-21` Handler sygnału `SIGTERM` — forceful shutdown z timeoutem drain 30s
- [x] `S0-22` Funkcja `drain_sessions(registry: &SessionRegistry, timeout: Duration)` — oczekiwanie na zamknięcie sesji

---

## Sprint 1 — Warstwa Transportowa i Demon SSH

> Cel: Asynchroniczny demon SSH obsługujący wiele sesji, bez dostępu do powłoki systemowej.  
> Stack: Rust + `russh` + `tokio`

### 1.1 Inicjalizacja projektu Cargo
- [x] `S1-01` Inicjalizacja workspace Cargo z podziałem na crate'y (`server`, `proto`, `renderer`)
- [x] `S1-02` Dodanie zależności: `russh`, `tokio`, `tokio-util`, `tracing`, `anyhow`

### 1.2 Struktura demona SSH
- [x] `S1-03` Definicja trait `OxiServer` implementującego `russh::server::Handler`
- [x] `S1-04` Funkcja `run_server(config: ServerConfig) -> Result<()>` — punkt wejścia demona
- [x] `S1-05` Struct `ServerConfig` — port, adres bind, ścieżka klucza hosta, limity sesji
- [x] `S1-06` Funkcja `load_host_key(path: &Path) -> Result<KeyPair>` — ładowanie klucza ed25519/RSA hosta

### 1.3 Autoryzacja kryptograficzna
- [x] `S1-07` Implementacja callbacku `auth_publickey` — weryfikacja klucza klienta (ed25519/RSA)
- [x] `S1-08` Funkcja `load_authorized_keys(path: &Path) -> Result<AuthorizedKeys>` — parsowanie `~/.ssh/authorized_keys`
- [x] `S1-09` Struct `AuthorizedKeys` z metodą `verify(key: &PublicKey) -> bool`
- [x] `S1-10` Implementacja callbacku `auth_password` — zawsze zwraca `Auth::Reject` (blokada haseł)

### 1.4 Obsługa PTY i sesji
- [x] `S1-11` Implementacja callbacku `pty_request` — przechwycenie wymiarów `(cols, rows)` okna
- [x] `S1-12` Struct `PtyDimensions { cols: u16, rows: u16 }` — przechowywanie wymiarów terminala
- [x] `S1-13` Implementacja callbacku `window_change_request` — aktualizacja `PtyDimensions` przy resize
- [x] `S1-14` Implementacja callbacku `channel_open_session` — tworzenie instancji sesji klienta
- [x] `S1-15` Struct `ClientSession` — id sesji, PTY dims, kanał wyjściowy, stan połączenia

### 1.5 Blokada powłoki systemowej
- [x] `S1-16` Implementacja callbacku `shell_request` — odrzucenie i podpięcie silnika renderującego zamiast bash
- [x] `S1-17` Implementacja callbacku `exec_request` — zawsze zwraca błąd (brak dostępu do exec)
- [x] `S1-18` Implementacja callbacku `subsystem_request` — blokada (tylko własne subsystemy OxiTerm)

### 1.6 Zarządzanie cyklem życia sesji
- [x] `S1-19` Funkcja `on_disconnect(session_id: SessionId)` — czyszczenie zasobów sesji
- [x] `S1-20` Struct `SessionRegistry` — thread-safe mapa `SessionId -> ClientSession` (Arc<RwLock<...>>)
- [x] `S1-21` Funkcja `graceful_shutdown(registry: &SessionRegistry)` — zamknięcie wszystkich sesji

### 1.7 Testy integracyjne Sprintu 1
- [x] `S1-22` Test: połączenie SSH con kluczem → sukces, bez dostępu do `ls`/`cd`
- [x] `S1-23` Test: połączenie SSH con hasłem → natychmiastowe odrzucenie
- [x] `S1-24` Test: resize okna → aktualizacja `PtyDimensions` w `ClientSession`

---

## Sprint 2 — Silnik THTML i Abstrakcyjne Drzewo Składniowe (AST)

> Cel: Deklaratywna reprezentacja interfejsu w pamięci serwera — szybkie drzewo AST z bezpieczną mutacją.

### 2.1 Definicja typów węzłów THTML
- [ ] `S2-01` Enum `NodeTag` — warianty: `Screen`, `Box`, `Text`, `Input`, `Button`, `Img`
- [ ] `S2-02` Struct `NodeAttributes` — pola: `id`, `class`, `style_raw`, `event_htmx`, `src` (dla Img)
- [ ] `S2-03` Struct `Node` — `tag: NodeTag`, `attrs: NodeAttributes`, `children: Vec<NodeId>`
- [ ] `S2-04` Typ `NodeId` — newtype over `u32`, unikalny identyfikator węzła w Arenie

### 2.2 Struktura Arena (pamięć węzłów)
- [ ] `S2-05` Struct `NodeArena` — wewnętrzny `Vec<Node>` z alokacją przez indeks `NodeId`
- [ ] `S2-06` Metoda `NodeArena::alloc(node: Node) -> NodeId` — alokacja węzła bez sterty runtime
- [ ] `S2-07` Metoda `NodeArena::get(id: NodeId) -> Option<&Node>` — dostęp przez id
- [ ] `S2-08` Metoda `NodeArena::get_mut(id: NodeId) -> Option<&mut Node>` — mutacja przez id
- [ ] `S2-09` Metoda `NodeArena::remove(id: NodeId)` — usuwanie węzła (lazy-mark)
- [ ] `S2-09b` Metoda `NodeArena::compact() -> RemapTable` — defragmentacja: przepakowanie aktywnych węzłów, zwrot mapy przesunięć `NodeId -> NodeId` (**wymagane gdy wypełnienie < 70%**)

### 2.3 Drzewo dokumentu
- [ ] `S2-10` Struct `THTMLDocument` — `arena: NodeArena`, `root: NodeId`, `dirty_nodes: Vec<NodeId>`
- [ ] `S2-11` Metoda `THTMLDocument::append_child(parent: NodeId, child: NodeId) -> Result<()>`
- [ ] `S2-12` Metoda `THTMLDocument::detach_child(parent: NodeId, child: NodeId) -> Result<()>`
- [ ] `S2-13` Metoda `THTMLDocument::mark_dirty(id: NodeId)` — oznaczenie węzła do re-renderingu

### 2.4 Parser THTML
- [ ] `S2-14` Funkcja `parse_thtml(input: &str) -> Result<THTMLDocument>` — punkt wejścia parsera
- [ ] `S2-15` Struct `THTMLParser` — wewnętrzny stan parsera (pozycja, stos tagów)
- [ ] `S2-16` Metoda `THTMLParser::parse_tag(&mut self) -> Result<NodeTag>` — rozpoznanie tagu
- [ ] `S2-17` Metoda `THTMLParser::parse_attributes(&mut self) -> Result<NodeAttributes>` — parsowanie atrybutów
- [ ] `S2-18` Metoda `THTMLParser::parse_text_content(&mut self) -> Result<String>` — treść tekstowa
- [ ] `S2-19` Metoda `THTMLParser::expect_close_tag(&mut self, tag: NodeTag) -> Result<()>` — walidacja zamknięcia
- [ ] `S2-20` Funkcja `reject_unknown_tag(name: &str) -> ParseError` — odrzucenie nieznanych tagów
- [ ] `S2-20b` Funkcja `sanitize_style_raw(raw: &str) -> Result<String, SanitizeError>` — usunięcie sekwencji ANSI / escape chars wstrzykniętych w wartość atrybutu `style_raw` (**wektor ataku: CSS injection**)

### 2.5 Serializacja i klonowanie stanu
- [ ] `S2-21` Trait `Serialize` dla `THTMLDocument` — zapis stanu do testów / snapshotów
- [ ] `S2-22` Metoda `THTMLDocument::clone_subtree(root: NodeId) -> THTMLDocument` — kopia poddrzewa

### 2.6 Testy jednostkowe Sprintu 2
- [ ] `S2-23` Test: parse poprawnego THTML → poprawne drzewo AST
- [ ] `S2-24` Test: nieznany tag → `ParseError`
- [ ] `S2-25` Test: `append_child` / `detach_child` → poprawna struktura dzieci
- [ ] `S2-26` Test: `mark_dirty` → węzeł trafia do `dirty_nodes`

---

## Sprint 3 — Silnik TCSS i Architektura Rysująca

> Cel: Parsowanie stylów, obliczanie layoutu Flexbox (Taffy), renderowanie do siatki komórek Cell, algorytm diff.

### 3.1 Parser TCSS
- [ ] `S3-01` Struct `StyleSheet` — kolekcja `Vec<StyleRule>`
- [ ] `S3-02` Struct `StyleRule` — `selector: Selector`, `declarations: Vec<Declaration>`
- [ ] `S3-03` Enum `Selector` — warianty: `Id(String)`, `Class(String)`, `Tag(NodeTag)`, `Universal`
- [ ] `S3-04` Struct `Declaration` — `property: CssProperty`, `value: CssValue`
- [ ] `S3-05` Enum `CssProperty` — warianty pokrywające TCSS: `Width`, `Height`, `Padding`, `Margin`, `Border`, `Color`, `Background`, `Display`, `AlignItems`, `JustifyContent`, `ZIndex`
- [ ] `S3-06` Enum `CssValue` — `Chars(u16)`, `Lines(u16)`, `Color(AnsiColor)`, `BorderStyle(BorderChars)`, `Auto`, `None`
- [ ] `S3-07` Funkcja `parse_tcss(input: &str) -> Result<StyleSheet>` — punkt wejścia parsera CSS
- [ ] `S3-08` Funkcja `apply_styles(doc: &THTMLDocument, sheet: &StyleSheet) -> ComputedStyles` — kaskadowe wiązanie styli

### 3.2 Typy kolorów i obramowań
- [ ] `S3-09` Enum `AnsiColor` — `TrueColor(u8, u8, u8)`, `Color256(u8)`, `Reset`
- [ ] `S3-10` Struct `BorderChars` — pola: `top_left`, `top`, `top_right`, `left`, `right`, `bot_left`, `bot`, `bot_right` (znaki Unicode Box Drawing)
- [ ] `S3-11` Funkcja `render_border(chars: &BorderChars, width: u16, height: u16) -> Vec<CellRow>` — renderuje ramkę do siatki

### 3.3 Silnik layoutu Flexbox (Taffy)
- [ ] `S3-12` Struct `LayoutEngine` — opakowanie na `taffy::Taffy` z mapą `NodeId -> taffy::NodeId`
- [ ] `S3-13` Metoda `LayoutEngine::build_tree(doc: &THTMLDocument, styles: &ComputedStyles)` — budowa drzewa Taffy z AST
- [ ] `S3-14` Metoda `LayoutEngine::compute(available: Size<u16>) -> Result<LayoutResult>` — obliczenie layoutu
- [ ] `S3-15` Struct `LayoutResult` — mapa `NodeId -> Rect` (pozycja x, y, width, height w jednostkach ch/lh)
- [ ] `S3-16` Funkcja `round_to_chars(f: f32) -> u16` — zaokrąglenie subpikseli do pełnych komórek znakowych
- [ ] `S3-17` Metoda `LayoutEngine::invalidate(node: NodeId)` — reset dirty node w drzewie Taffy (dirty flagging)

### 3.4 Siatka komórek (Cell Buffer)
- [ ] `S3-18` Struct `Cell` — `ch: char`, `fg: AnsiColor`, `bg: AnsiColor`, `modifiers: CellModifiers`
- [ ] `S3-19` Struct `CellModifiers` — `bold: bool`, `underline: bool`, `italic: bool`
- [ ] `S3-20` Struct `CellBuffer` — `cells: Vec<Cell>` (**flat layout**, nie Vec<Vec<Cell>>), `width: u16`, `height: u16` — indeksowanie: `idx = y * width + x`
- [ ] `S3-20b` Funkcja `flat_idx(x: u16, y: u16, width: u16) -> usize` — inline helper dla bezpiecznego obliczania indeksu w flat buforze
- [ ] `S3-21` Metoda `CellBuffer::new(width: u16, height: u16) -> CellBuffer` — inicjalizacja `Vec<Cell>` o pojemności `width * height`
- [ ] `S3-22` Metoda `CellBuffer::set(x: u16, y: u16, cell: Cell)` — zapis przez `flat_idx`, bounds check w debug mode
- [ ] `S3-23` Metoda `CellBuffer::clear()` — reset do spacji z domyślnymi kolorami

### 3.5 Renderer AST → CellBuffer
- [ ] `S3-24` Struct `Renderer` — przyjmuje `LayoutResult` i `ComputedStyles`, produkuje `CellBuffer`
- [ ] `S3-25` Metoda `Renderer::render_node(node: NodeId, buf: &mut CellBuffer)` — rekurencyjne malowanie węzła
- [ ] `S3-26` Metoda `Renderer::render_text(text: &str, rect: Rect, buf: &mut CellBuffer)` — zawijanie tekstu w rect
- [ ] `S3-27` Metoda `Renderer::render_border(rect: Rect, style: &BorderStyle, buf: &mut CellBuffer)` — obramowanie Unicode

### 3.6 Algorytm różnicowy (Diff Engine)
- [ ] `S3-28` Struct `DiffEngine` — `prev: CellBuffer`, `next: CellBuffer`
- [ ] `S3-29` Metoda `DiffEngine::diff() -> Vec<AnsiCommand>` — porównanie buforów, generacja komend ANSI
- [ ] `S3-30` Enum `AnsiCommand` — `MoveCursor(u16, u16)`, `SetColor(AnsiColor, AnsiColor)`, `WriteChar(char)`, `SetModifiers(CellModifiers)`
- [ ] `S3-31` Funkcja `encode_ansi(commands: &[AnsiCommand]) -> Vec<u8>` — serializacja do bajtów ANSI
- [ ] `S3-32` Funkcja `bypass_diff_region(buf: &mut CellBuffer, rect: Rect)` — pominięcie diff dla statycznych bloków (Bypass Diff)

### 3.7 Podwójne buforowanie
- [ ] `S3-33` Struct `DoubleBuffer` — `front: CellBuffer`, `back: CellBuffer`
- [ ] `S3-34` Metoda `DoubleBuffer::swap()` — zamiana front/back po wysłaniu diff
- [ ] `S3-35` Metoda `DoubleBuffer::emit_diff() -> Vec<u8>` — oblicz diff, enkoduj ANSI, swap

### 3.8 Testy Sprintu 3
- [ ] `S3-36` Test: parse TCSS → poprawny `StyleSheet`
- [ ] `S3-37` Test: `LayoutEngine::compute` → poprawne `Rect` dla układu flex
- [ ] `S3-38` Test: `Renderer::render_node` → poprawna zawartość `CellBuffer`
- [ ] `S3-39` Test: `DiffEngine::diff` → minimalna lista `AnsiCommand` przy jednej zmianie
- [ ] `S3-40` Test: `encode_ansi` → poprawne sekwencje bajtów ANSI


---

## Sprint 4 — Interaktywność i Mechanika Zdarzeń HTMX

> Cel: Obsługa myszy, klawiatury (Kitty Protocol), hit-testing, asynchroniczne callbacki HTMX.  
> Wzorzec: Resilient Reactor Thread (RRT) + kanały mpsc

### 4.1 Wzorzec Resilient Reactor Thread (RRT)
- [ ] `S4-01` Struct `ReactorThread` — dedykowany OS thread na nasłuchiwanie I/O z PTY/SSH
- [ ] `S4-02` Funkcja `ReactorThread::spawn(pty_fd: RawFd, tx: mpsc::Sender<InputEvent>) -> JoinHandle` — uruchomienie wątku
- [ ] `S4-03` Pętla `ReactorThread::run_loop` — blokujący `epoll`/`mio::Poll` na deskryptorze
- [ ] `S4-04` Funkcja `ReactorThread::sanitize_frame(raw: &[u8]) -> Option<Vec<u8>>` — odrzucenie zdeformowanych sekwencji

### 4.2 Dekoder zdarzeń wejściowych
- [ ] `S4-05` Enum `InputEvent` — `KeyPress(KeyEvent)`, `MouseEvent(MouseInput)`, `Resize(PtyDimensions)`, `Unknown`
- [ ] `S4-06` Struct `KeyEvent` — `codepoint: u32`, `modifiers: KeyModifiers`, `kind: KeyKind`
- [ ] `S4-07` Enum `KeyKind` — `Press`, `Repeat`, `Release`
- [ ] `S4-08` Struct `KeyModifiers` — `shift`, `ctrl`, `alt`, `super_key`, `hyper`, `meta`, `caps_lock` (bitmapa)
- [ ] `S4-09` Struct `MouseInput` — `col: u16`, `row: u16`, `button: MouseButton`, `action: MouseAction`
- [ ] `S4-10` Enum `MouseButton` — `Left`, `Middle`, `Right`, `WheelUp`, `WheelDown`
- [ ] `S4-11` Enum `MouseAction` — `Press`, `Release`, `Move`
- [ ] `S4-12` Funkcja `decode_input(buf: &[u8]) -> Result<InputEvent>` — główny dekoder strumienia

### 4.3 Kitty Keyboard Protocol
- [ ] `S4-13` Funkcja `enable_kitty_protocol(writer: &mut impl Write) -> Result<()>` — wysłanie `CSI = 1 u` do emulatora
- [ ] `S4-14` Funkcja `disable_kitty_protocol(writer: &mut impl Write) -> Result<()>` — wysłanie `CSI < u`
- [ ] `S4-15` Funkcja `parse_kitty_key(seq: &[u8]) -> Option<KeyEvent>` — parsowanie sekwencji `CSI u`

### 4.4 Protokół SGR 1006 (fallback myszy)
- [ ] `S4-16` Funkcja `enable_sgr_mouse(writer: &mut impl Write) -> Result<()>` — `CSI ? 1006 h`
- [ ] `S4-17` Funkcja `disable_sgr_mouse(writer: &mut impl Write) -> Result<()>` — `CSI ? 1006 l`
- [ ] `S4-18` Funkcja `parse_sgr_mouse(seq: &[u8]) -> Option<MouseInput>` — parsowanie `CSI < ... M/m`
- [ ] `S4-19` Funkcja `sgr_timeout_guard(seq_buf: &mut Vec<u8>, deadline: Instant) -> bool` — odrzucenie niekompletnych sekwencji po timeout

### 4.5 Hit-Testing
- [ ] `S4-20` Struct `HitTester` — przyjmuje `LayoutResult`, pozwala na zapytania o węzeł pod kursorem
- [ ] `S4-21` Metoda `HitTester::find_node(col: u16, row: u16) -> Option<NodeId>` — wyszukiwanie węzła po współrzędnych
- [ ] `S4-22` Metoda `HitTester::is_interactive(node: NodeId) -> bool` — sprawdzenie czy węzeł ma event HTMX

### 4.6 System zdarzeń HTMX (callbacki)
- [ ] `S4-23` Trait `EventHandler` — metoda `handle(event: &HtmxEvent, doc: &mut THTMLDocument) -> Result<()>`
- [ ] `S4-24` Enum `HtmxEvent` — `Click(NodeId)`, `Input(NodeId, String)`, `Focus(NodeId)`, `Blur(NodeId)`
- [ ] `S4-25` Struct `EventBus` — rejestr `NodeId -> Box<dyn EventHandler>`
- [ ] `S4-26` Metoda `EventBus::register(node: NodeId, handler: Box<dyn EventHandler>)`
- [ ] `S4-27` Metoda `EventBus::dispatch(event: HtmxEvent, doc: &mut THTMLDocument) -> Result<()>`
- [ ] `S4-28` Funkcja `partial_update(doc: &mut THTMLDocument, changed_nodes: &[NodeId])` — re-ewaluacja tylko zmienionych węzłów

### 4.7 Pętla zdarzeń serwera
- [ ] `S4-29` Struct `EventLoop` — konsumuje `mpsc::Receiver<InputEvent>`, wywołuje hit-test + dispatch
- [ ] `S4-30` Metoda `EventLoop::run(&mut self) -> Result<()>` — główna pętla `async` w Tokio
- [ ] `S4-31` Funkcja `debounce_resize(rx: &mut Receiver<PtyDimensions>, window_ms: u64) -> PtyDimensions` — buforowanie sygnałów SIGWINCH

### 4.8 Testy Sprintu 4
- [ ] `S4-32` Test: `parse_kitty_key` — poprawne rozpoznanie key press/release z modyfikatorami
- [ ] `S4-33` Test: `parse_sgr_mouse` — poprawne współrzędne i button przy sfragmentowanym buforze
- [ ] `S4-34` Test: `HitTester::find_node` — poprawny `NodeId` dla kliknięcia w obszar przycisku
- [ ] `S4-35` Test: `EventBus::dispatch` → wywołanie handlera + mutacja `THTMLDocument`

---

## Sprint 5 — Optymalizacja, Capability Negotiation i Backpressure

> Cel: Adaptacja do możliwości emulatora, kontrola przepływu, latency mitigation, Unicode width stabilization.

### 5.1 Negocjacja możliwości emulatora (Capability Negotiation)
- [ ] `S5-01` Funkcja `send_da1_query(writer: &mut impl Write) -> Result<()>` — wysłanie `ESC [c`
- [ ] `S5-02` Funkcja `parse_da1_response(buf: &[u8]) -> TerminalProfile` — parsowanie odpowiedzi emulatora
- [ ] `S5-03` Struct `TerminalProfile` — `supports_kitty_kbd: bool`, `supports_kitty_gfx: bool`, `supports_sgr_mouse: bool`, `unicode_version: u8`, `color_depth: ColorDepth`
- [ ] `S5-04` Enum `ColorDepth` — `TrueColor`, `Color256`, `Color16`
- [ ] `S5-05` Funkcja `negotiate_capabilities(session: &mut ClientSession) -> Result<TerminalProfile>` — pełny handshake z timeoutem

### 5.2 Adaptacyjny tryb renderowania
- [ ] `S5-06` Enum `RenderMode` — `Full60fps`, `Degraded30fps`, `Minimal`
- [ ] `S5-07` Funkcja `select_render_mode(profile: &TerminalProfile, rtt_ms: u32) -> RenderMode` — wybór trybu na podstawie profilu i latencji
- [ ] `S5-08` Struct `FrameRateLimiter` — `target_fps: u8`, `last_frame: Instant`
- [ ] `S5-09` Metoda `FrameRateLimiter::should_render() -> bool` — sprawdzenie czy czas na nową klatkę
- [ ] `S5-10` Metoda `FrameRateLimiter::frame_drop(doc: &THTMLDocument) -> CellBuffer` — pominięcie klatek pośrednich, bezpośredni skok do aktualnego stanu

### 5.3 Synchronized Updates (BSU/ESU, tearing prevention)
- [ ] `S5-11` Funkcja `send_bsu(writer: &mut impl Write) -> Result<()>` — `CSI ? 2026 h` (Begin Synchronized Update)
- [ ] `S5-12` Funkcja `send_esu(writer: &mut impl Write) -> Result<()>` — `CSI ? 2026 l` (End Synchronized Update)
- [ ] `S5-13` Struct `SyncedEmitter` — opakowuje writer, automatycznie BSU/ESU przy emit_diff

### 5.4 Backpressure i kontrola przepływu
- [ ] `S5-14` Struct `BoundedFrameChannel` — `sender: mpsc::SyncSender<Vec<u8>>`, pojemność = 2 klatki
- [ ] `S5-15` Metoda `BoundedFrameChannel::try_send(frame: Vec<u8>) -> SendResult` — nieblokujące wysłanie lub load-shed
- [ ] `S5-16` Enum `SendResult` — `Sent`, `Dropped`, `Blocked`
- [ ] `S5-17` Funkcja `poll_ready(writer: &TcpStream) -> bool` — sprawdzenie czy bufor TCP klienta jest gotowy

### 5.5 XON/XOFF In-Band Flow Control
- [ ] `S5-18` Funkcja `handle_xoff(session: &mut ClientSession)` — zatrzymanie wysyłania po odebraniu `DC3 (0x13)`
- [ ] `S5-19` Funkcja `handle_xon(session: &mut ClientSession)` — wznowienie wysyłania po `DC1 (0x11)`
- [ ] `S5-20` Metoda `ReactorThread::detect_flow_control(byte: u8) -> Option<FlowSignal>` — detekcja XON/XOFF w strumieniu

### 5.6 Lokalne echo predykcyjne (Mosh-style)
- [ ] `S5-21` Struct `PredictiveEcho` — bufor przewidywanych znaków z flagą `confirmed: bool`
- [ ] `S5-22` Metoda `PredictiveEcho::predict(ch: char, node: NodeId)` — rysowanie znaku z atrybutem "predykcja" (underline)
- [ ] `S5-23` Metoda `PredictiveEcho::confirm(server_state: &CellBuffer)` — porównanie z autorytatywnym stanem, cofnięcie przy konflikcie
- [ ] `S5-24` Metoda `PredictiveEcho::flush_to_server(tx: &mpsc::Sender<InputEvent>)` — wysłanie zbuforowanego tekstu do serwera
- [ ] `S5-25` Funkcja `rtt_detector(session: &ClientSession) -> u32` — pomiar RTT, decyzja o włączeniu predykcji

### 5.7 Stabilizacja szerokości Unicode
- [ ] `S5-26` Funkcja `send_unicode_version_osc(writer: &mut impl Write, version: u8) -> Result<()>` — OSC 1337 ; UnicodeVersion=N
- [ ] `S5-27` Struct `UnicodeWidthCache` — cache `char -> u8` dla wyników `unicode_width`
- [ ] `S5-28` Metoda `UnicodeWidthCache::width(ch: char) -> u8` — lookup z cache, fallback do biblioteki
- [ ] `S5-29` Funkcja `insert_vtm_modifier(buf: &mut Vec<u8>, cluster_width: u8)` — wstawienie modyfikatora PUA (U+D0000–U+D08F6) po klastrze grafemowym

### 5.8 Debouncing SIGWINCH
- [ ] `S5-30` Struct `ResizeDebouncer` — `pending: Option<PtyDimensions>`, `deadline: Option<Instant>`, `window_ms: u64`
- [ ] `S5-31` Metoda `ResizeDebouncer::push(dims: PtyDimensions)` — akumulacja sygnałów resize
- [ ] `S5-32` Metoda `ResizeDebouncer::poll() -> Option<PtyDimensions>` — zwróc stabilny rozmiar po upływie okna

### 5.9 Testy Sprintu 5
- [ ] `S5-33` Test: `negotiate_capabilities` → poprawny `TerminalProfile` dla GNOME Terminal / Alacritty / Ghostty
- [ ] `S5-34` Test: `BoundedFrameChannel::try_send` przy pełnym kanale → `SendResult::Dropped`
- [ ] `S5-35` Test: `PredictiveEcho::confirm` przy konflikcie → cofnięcie predykcji
- [ ] `S5-36` Test: `ResizeDebouncer` → pojedynczy callback po serii sygnałów SIGWINCH

---

## Sprint 6 — Grafika (Kitty/Sixel), Bezpieczeństwo i Dostępność (A11y / AT-SPI2)

> Cel: Obsługa `<img>`, defensywne parsowanie, tunelowanie D-Bus dla Orca.

### 6.1 Detekcja obsługi grafiki
- [ ] `S6-01` Funkcja `probe_kitty_graphics(writer: &mut impl Write) -> Result<()>` — wysłanie testowego APC, oczekiwanie na ACK
- [ ] `S6-02` Funkcja `parse_kitty_ack(buf: &[u8]) -> bool` — rozpoznanie odpowiedzi `OK` od emulatora
- [ ] `S6-03` Funkcja `probe_sixel_support(profile: &TerminalProfile) -> bool` — sprawdzenie flagi w `TerminalProfile`

### 6.2 Kitty Graphics Protocol
- [ ] `S6-04` Struct `KittyImageManager` — rejestr `image_id: u32 -> KittyCachedImage`
- [ ] `S6-05` Struct `KittyCachedImage` — `id: u32`, `width: u32`, `height: u32`, `uploaded: bool`
- [ ] `S6-06` Metoda `KittyImageManager::upload(id: u32, png_data: &[u8], writer: &mut impl Write) -> Result<()>` — chunked Base64 upload przez APC
- [ ] `S6-07` Funkcja `encode_kitty_chunk(id: u32, chunk: &[u8], more: bool) -> Vec<u8>` — enkodowanie pojedynczego chunka APC
- [ ] `S6-08` Metoda `KittyImageManager::place(id: u32, x: u16, y: u16, writer: &mut impl Write) -> Result<()>` — rysowanie z cache terminala (bez ponownego uploadu)
- [ ] `S6-09` Metoda `KittyImageManager::evict(id: u32, writer: &mut impl Write) -> Result<()>` — usunięcie obrazu z cache terminala

### 6.3 Sixel (fallback)
- [ ] `S6-10` Funkcja `encode_sixel(img: &RgbaImage, palette_size: u8) -> Vec<u8>` — konwersja obrazu do strumienia Sixel z kwantyzacją
- [ ] `S6-11` Funkcja `sixel_rle_compress(data: &[u8]) -> Vec<u8>` — kompresja Run-Length Encoding dla Sixel
- [ ] `S6-12` Funkcja `dither_image(img: &RgbaImage, colors: u8) -> RgbaImage` — bezszumna kwantyzacja dla fallbacku Unicode-block

### 6.4 Renderer węzła `<img>`
- [ ] `S6-13` Metoda `Renderer::render_img(node: NodeId, rect: Rect, buf: &mut CellBuffer, writer: &mut impl Write)` — dispatching do Kitty/Sixel/Unicode-block
- [ ] `S6-14` Funkcja `unicode_block_fallback(img: &RgbaImage, rect: Rect, buf: &mut CellBuffer)` — renderowanie przez znaki `▄`/`▀` gdy brak wsparcia graficznego

### 6.5 Defensive Parsing — granica bezpieczeństwa
- [ ] `S6-15` Struct `BoundedSubnegBuffer` — bufor o stałej pojemności (max 256 bajtów)
- [ ] `S6-16` Metoda `BoundedSubnegBuffer::push(byte: u8) -> Result<(), OverflowError>` — odrzucenie po przekroczeniu limitu
- [ ] `S6-17` Struct `InputStateMachine` — automat skończony parsujący sekwencje ANSI/Kitty/SGR
- [ ] `S6-18` Metoda `InputStateMachine::feed(byte: u8) -> Option<InputEvent>` — krok automatu, panic-free
- [ ] `S6-19` Metoda `InputStateMachine::reset()` — reset do stanu `Idle` przy błędzie lub przepełnieniu
- [ ] `S6-20` Funkcja `validate_thtml_attribute(key: &str, value: &str) -> Result<(), SanitizeError>` — walidacja atrybutów THTML przed insertem do AST

### 6.6 Dostępność — tunelowanie AT-SPI2 przez SSH
- [ ] `S6-21` Struct `A11yNode` — semantyczne odwzorowanie `Node` z AST: `role: AtSpRole`, `label: String`, `value: Option<String>`
- [ ] `S6-22` Enum `AtSpRole` — `Button`, `TextInput`, `Label`, `Container`, `Image`
- [ ] `S6-23` Funkcja `build_a11y_tree(doc: &THTMLDocument) -> Vec<A11yNode>` — mapowanie AST na drzewo AT-SPI
- [ ] `S6-24` Struct `DBusBridge` — zarządza tunelem gniazda AF_UNIX przez SSH port forwarding
- [ ] `S6-25` Metoda `DBusBridge::read_dbus_address() -> Result<String>` — odczyt `DBUS_SESSION_BUS_ADDRESS` na serwerze
- [ ] `S6-26` Metoda `DBusBridge::open_tunnel(local_path: &Path, remote_path: &Path) -> Result<()>` — wywołanie `ssh -L` dla gniazda D-Bus
- [ ] `S6-27` Metoda `DBusBridge::register_at_spi(tree: &[A11yNode]) -> Result<()>` — rejestracja węzłów przez AT-SPI2 po tunelu
- [ ] `S6-28` Metoda `DBusBridge::update_focus(node: NodeId, tree: &[A11yNode]) -> Result<()>` — powiadomienie Orki o zmianie focusu

### 6.7 Tryb Fallback liniowy (A11y safe mode)
- [ ] `S6-29` Funkcja `detect_a11y_mode(args: &[String]) -> bool` — wykrywanie flagi `--a11y` przy połączeniu
- [ ] `S6-30` Funkcja `render_linear_fallback(doc: &THTMLDocument) -> String` — spłaszczenie AST do semantycznego tekstu liniowego
- [ ] `S6-31` Funkcja `emit_linear_stream(text: &str, writer: &mut impl Write) -> Result<()>` — wysłanie tekstu do klienta bez TCSS/Flexbox

### 6.8 Testy Sprintu 6
- [ ] `S6-32` Test: `KittyImageManager::upload` → poprawne chunki APC Base64
- [ ] `S6-33` Test: `encode_sixel` → poprawny strumień Sixel dla obrazu 2x2 px
- [ ] `S6-34` Test: `BoundedSubnegBuffer::push` powyżej 256 bajtów → `OverflowError` + reset automatu
- [ ] `S6-35` Test: `InputStateMachine::feed` na zdeformowanej sekwencji → `InputEvent::Unknown`, brak paniki
- [ ] `S6-36` Test: `build_a11y_tree` → poprawne role AT-SPI dla przycisków i inputów
- [ ] `S6-37` Test: `render_linear_fallback` → płaski tekst semantyczny bez znaków Box Drawing
