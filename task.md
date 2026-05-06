# OxiTerm — Task Backlog

> Granularność: 1 funkcja / 1 obiekt = 1 task  
> Status: `[ ]` todo · `[/]` in progress · `[x]` done

---

## Sprint 1 — Warstwa Transportowa i Demon SSH

> Cel: Asynchroniczny demon SSH obsługujący wiele sesji, bez dostępu do powłoki systemowej.  
> Stack: Rust + `russh` + `tokio`

### 1.1 Inicjalizacja projektu Cargo
- [ ] `S1-01` Inicjalizacja workspace Cargo z podziałem na crate'y (`server`, `proto`, `renderer`)
- [ ] `S1-02` Dodanie zależności: `russh`, `tokio`, `tokio-util`, `tracing`, `anyhow`

### 1.2 Struktura demona SSH
- [ ] `S1-03` Definicja trait `OxiServer` implementującego `russh::server::Handler`
- [ ] `S1-04` Funkcja `run_server(config: ServerConfig) -> Result<()>` — punkt wejścia demona
- [ ] `S1-05` Struct `ServerConfig` — port, adres bind, ścieżka klucza hosta, limity sesji
- [ ] `S1-06` Funkcja `load_host_key(path: &Path) -> Result<KeyPair>` — ładowanie klucza ed25519/RSA hosta

### 1.3 Autoryzacja kryptograficzna
- [ ] `S1-07` Implementacja callbacku `auth_publickey` — weryfikacja klucza klienta (ed25519/RSA)
- [ ] `S1-08` Funkcja `load_authorized_keys(path: &Path) -> Result<AuthorizedKeys>` — parsowanie `~/.ssh/authorized_keys`
- [ ] `S1-09` Struct `AuthorizedKeys` z metodą `verify(key: &PublicKey) -> bool`
- [ ] `S1-10` Implementacja callbacku `auth_password` — zawsze zwraca `Auth::Reject` (blokada haseł)

### 1.4 Obsługa PTY i sesji
- [ ] `S1-11` Implementacja callbacku `pty_request` — przechwycenie wymiarów `(cols, rows)` okna
- [ ] `S1-12` Struct `PtyDimensions { cols: u16, rows: u16 }` — przechowywanie wymiarów terminala
- [ ] `S1-13` Implementacja callbacku `window_change_request` — aktualizacja `PtyDimensions` przy resize
- [ ] `S1-14` Implementacja callbacku `channel_open_session` — tworzenie instancji sesji klienta
- [ ] `S1-15` Struct `ClientSession` — id sesji, PTY dims, kanał wyjściowy, stan połączenia

### 1.5 Blokada powłoki systemowej
- [ ] `S1-16` Implementacja callbacku `shell_request` — odrzucenie i podpięcie silnika renderującego zamiast bash
- [ ] `S1-17` Implementacja callbacku `exec_request` — zawsze zwraca błąd (brak dostępu do exec)
- [ ] `S1-18` Implementacja callbacku `subsystem_request` — blokada (tylko własne subsystemy OxiTerm)

### 1.6 Zarządzanie cyklem życia sesji
- [ ] `S1-19` Funkcja `on_disconnect(session_id: SessionId)` — czyszczenie zasobów sesji
- [ ] `S1-20` Struct `SessionRegistry` — thread-safe mapa `SessionId -> ClientSession` (Arc<RwLock<...>>)
- [ ] `S1-21` Funkcja `graceful_shutdown(registry: &SessionRegistry)` — zamknięcie wszystkich sesji

### 1.7 Testy integracyjne Sprintu 1
- [ ] `S1-22` Test: połączenie SSH z kluczem → sukces, bez dostępu do `ls`/`cd`
- [ ] `S1-23` Test: połączenie SSH z hasłem → natychmiastowe odrzucenie
- [ ] `S1-24` Test: resize okna → aktualizacja `PtyDimensions` w `ClientSession`

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

### 2.3 Drzewo dokumentu
- [ ] `S2-10` Struct `THTMLDocument` — `arena: NodeArena`, `root: NodeId`, `dirty_nodes: Vec<NodeId>`
- [ ] `S2-11` Metoda `THTMLDocument::append_child(parent: NodeId, child: NodeId) -> Result<()>`
- [ ] `S2-12` Metoda `THTMLDocument::detach_child(parent: NodeId, child: NodeId) -> Result<()>`
- [ ] `S2-13` Metoda `THTMLDocument::mark_dirty(id: NodeId)` — oznaczenie węzła do re-renderingu

### 2.4 Parser THTML
- [ ] `S2-14` Funkcja `parse_thtml(input: &str) -> Result<THTMLDocument>` — punkt wejścia parsera
- [ ] `S2-15` Struct `THTMLParser` — wewnętrzny stan parsera (pozycja, stos tagów)
- [ ] `S2-16` Metoda `THTMLParser::parse_tag(&mut self) -> Result<NodeTag>` — rozpoznanie tagu
- [ ] `S2-17` Metoda `THTMLParser::parse_attributes(&mut self) -> Result<NodeAttributes>` — parsowanie atrybutuów
- [ ] `S2-18` Metoda `THTMLParser::parse_text_content(&mut self) -> Result<String>` — treść tekstowa
- [ ] `S2-19` Metoda `THTMLParser::expect_close_tag(&mut self, tag: NodeTag) -> Result<()>` — walidacja zamknięcia
- [ ] `S2-20` Funkcja `reject_unknown_tag(name: &str) -> ParseError` — odrzucenie nieznanych tagów

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
- [ ] `S3-20` Struct `CellBuffer` — `cells: Vec<Vec<Cell>>`, `width: u16`, `height: u16`
- [ ] `S3-21` Metoda `CellBuffer::new(width: u16, height: u16) -> CellBuffer` — inicjalizacja pustą siatką
- [ ] `S3-22` Metoda `CellBuffer::set(x: u16, y: u16, cell: Cell)` — zapis komórki
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
