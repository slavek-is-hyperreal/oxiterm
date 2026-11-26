//! Cache registries for media playback and graphics generation.
//!
//! Provides FIFO-eviction caches for images, SVG trees, Lottie animations, and
//! processes video rendering streams via external tools such as FFmpeg.
//!
//! Everything that needs threads, the filesystem, a clock or C bindings (rlottie, resvg, ffmpeg)
//! is behind the `native-media` feature; `PlaybackRegistry`'s pure toggle/hover state (`.riv`)
//! is always available.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
#[cfg(feature = "native-media")]
use std::sync::{Arc, RwLock};

/// Cache lookup key for rendered graphic assets.
#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct CacheKey {
    /// Absolute or relative path to the source file.
    pub path: PathBuf,
    /// Requested output width in pixels.
    pub width_px: u32,
    /// Requested output height in pixels.
    pub height_px: u32,
    /// Optional frame index (used for animations/video files).
    pub frame_idx: Option<usize>,
}

/// The serialization format used to emit graphics in terminal-compatible sequences.
#[derive(Clone, Debug)]
pub enum GraphicFormat {
    /// Sixel graphic format stream sequence.
    Sixel(Vec<u8>),
    /// Kitty Graphics Protocol escape command sequence.
    Kitty(Vec<u8>),
}

/// Cached graphical representation value.
#[derive(Clone, Debug)]
pub struct CacheValue {
    /// Format and payload byte sequence.
    pub format: GraphicFormat,
}

/// A wrapper around rlottie `Animation` that makes it safe to share across threads.
#[cfg(feature = "native-media")]
pub struct SafeAnimation {
    /// The underlying rlottie Animation instance.
    pub anim: rlottie::Animation,
}
#[cfg(feature = "native-media")]
unsafe impl Send for SafeAnimation {}

/// Playback tracking state of an animated or interactive media asset.
#[derive(Clone)]
pub struct PlaybackState {
    /// Timestamp when media rendering started (native media only: lottie frame pacing; the pure
    /// build has no clock).
    #[cfg(feature = "native-media")]
    pub start_time: std::time::Instant,
    /// Is the user cursor currently hovering over this element.
    pub hover: bool,
    /// Is the element currently clicked or pressed.
    pub click_active: bool,
    /// Coordinates of the last click (column, row).
    pub click_coord: Option<(u16, u16)>,
    /// State toggle switcher state (e.g. play/pause).
    pub toggled: bool,
}

/// Registry storing the state of playing media instances, **per owner** (BAD ACTOR 7.6 F-12): every
/// method takes a `scope` (the session id; see `TerminalProfile::playback_scope`), so one visitor's
/// hover, click or toggle is never seen by another session showing the same file. Only
/// content-derived caches (the Lottie parse, the Rive raster, `AssetCache`) are shared.
pub struct PlaybackRegistry {
    states: Mutex<HashMap<(u64, PathBuf), PlaybackState>>,
}

impl PlaybackRegistry {
    /// Entries kept in all; at the cap the entries of every other scope are dropped.
    const MAX_ENTRIES: usize = 4096;

    /// Returns the global thread-safe singleton instance of the registry.
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<PlaybackRegistry> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            states: Mutex::new(HashMap::new()),
        })
    }

    fn fresh() -> PlaybackState {
        PlaybackState {
            #[cfg(feature = "native-media")]
            start_time: std::time::Instant::now(),
            hover: false,
            click_active: false,
            click_coord: None,
            toggled: false,
        }
    }

    /// Fetches the playback state of `path` for `scope`, creating a default one if missing.
    pub fn get_or_create(&self, scope: u64, path: &Path) -> PlaybackState {
        let mut lock = self.states.lock().unwrap();
        let key = (scope, path.to_path_buf());
        if let Some(state) = lock.get(&key) {
            return state.clone();
        }
        if lock.len() >= Self::MAX_ENTRIES {
            lock.retain(|(s, _), _| *s == scope);
            if lock.len() >= Self::MAX_ENTRIES {
                lock.clear();
            }
        }
        let state = Self::fresh();
        lock.insert(key, state.clone());
        state
    }

    /// Forgets everything of a scope (its session ended).
    pub fn drop_scope(&self, scope: u64) {
        self.states.lock().unwrap().retain(|(s, _), _| *s != scope);
    }

    /// Fetches or parses a Lottie animation (content cache shared by every session: see
    /// [`LottieCache`]); `None` when the file is missing or rejected.
    #[cfg(feature = "native-media")]
    pub fn get_or_load_lottie(&self, path: &Path) -> Option<Arc<Mutex<SafeAnimation>>> {
        LottieCache::get().get_or_load(path)
    }

    /// Sets the hover flag of a registered playback state.
    pub fn set_hover(&self, scope: u64, path: &Path, hover: bool) {
        let mut lock = self.states.lock().unwrap();
        if let Some(state) = lock.get_mut(&(scope, path.to_path_buf())) {
            state.hover = hover;
        }
    }

    /// Sets the click state parameters of a registered playback instance.
    pub fn set_click(&self, scope: u64, path: &Path, active: bool, coord: Option<(u16, u16)>) {
        let mut lock = self.states.lock().unwrap();
        if let Some(state) = lock.get_mut(&(scope, path.to_path_buf())) {
            state.click_active = active;
            state.click_coord = coord;
            if active {
                state.toggled = !state.toggled;
            }
        }
    }
}

/// Largest Lottie file parsed (a parsed JSON tree takes several times its size).
#[cfg(feature = "native-media")]
pub const MAX_LOTTIE_BYTES: u64 = 4 * 1024 * 1024;

/// Most vertices in one bezier path, and in the whole document.
#[cfg(feature = "native-media")]
pub const MAX_PATH_VERTICES: usize = 5_000;
#[cfg(feature = "native-media")]
pub const MAX_TOTAL_VERTICES: usize = 40_000;

/// Why a Lottie document was not handed to rlottie.
#[cfg(feature = "native-media")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LottieReject {
    /// Larger than [`MAX_LOTTIE_BYTES`].
    TooLarge,
    /// Not JSON, nested too deeply, or not a Lottie object.
    Malformed,
    /// A precomp refers (directly or through others) to itself, or nests too deeply.
    RefCycle,
    /// Dimensions, duration, layer count or expanded layer count out of bounds.
    TooComplex(&'static str),
    /// rlottie refused it.
    Engine,
}

/// Checks a Lottie document before rlottie sees it. rlottie (C++) aborts the whole process on a
/// self-referencing precomp and spends unbounded time on huge shapes; neither can be caught
/// in-process, so they are rejected here from the parsed JSON (BAD ACTOR 7.6 F-06):
///
/// * size <= [`MAX_LOTTIE_BYTES`] and JSON nesting within serde_json's recursion limit;
/// * `w`/`h` finite in `1..=4096`, `fr` finite and positive, `ip`/`op` finite, at most 1e6 frames;
/// * at most 5 000 vertices per bezier path and 40 000 in all (measured: 10 000 in one path 0.06 s, 20 000 > 100 s),
///   at most 1 000 star points;
/// * at most 20 000 layers in total, asset ids unique, `refId` references acyclic (checked from every
///   asset, reachable or not), nesting depth <= 64 (a deeper chain is reported as `RefCycle`), and at most
///   200 000 layer instances once precomps are expanded (a DAG that references the next precomp
///   twice per level would otherwise be exponential).
#[cfg(feature = "native-media")]
pub fn validate_lottie(data: &str) -> Result<(), LottieReject> {
    use serde_json::Value;
    if data.len() as u64 > MAX_LOTTIE_BYTES {
        return Err(LottieReject::TooLarge);
    }
    let v: Value = serde_json::from_str(data).map_err(|_| LottieReject::Malformed)?;
    let root = v.as_object().ok_or(LottieReject::Malformed)?;
    let num = |k: &str| root.get(k).and_then(Value::as_f64);
    let (w, h) = (num("w").ok_or(LottieReject::Malformed)?, num("h").ok_or(LottieReject::Malformed)?);
    if !(1.0..=4096.0).contains(&w) || !(1.0..=4096.0).contains(&h) {
        return Err(LottieReject::TooComplex("size"));
    }
    let fr = num("fr").ok_or(LottieReject::Malformed)?;
    let (ip, op) = (num("ip").ok_or(LottieReject::Malformed)?, num("op").ok_or(LottieReject::Malformed)?);
    if !(fr.is_finite() && fr > 0.0 && ip.is_finite() && op.is_finite()) || (op - ip).abs() > 1e6 {
        return Err(LottieReject::TooComplex("time"));
    }
    let layers_of = |val: &Value| -> Vec<Value> { val.get("layers").and_then(Value::as_array).cloned().unwrap_or_default() };
    // Path complexity: rlottie's rasteriser has a cliff (10k vertices in one path 0.06 s, 20k more
    // than 100 s). Bezier paths are objects with parallel `v`/`i`/`o` arrays; keyframed paths repeat
    // them per keyframe, so every one is counted.
    let (mut vertices, mut stack) = (0usize, vec![&v]);
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(m) => {
                if let (Some(Value::Array(vs)), Some(_), Some(_)) = (m.get("v"), m.get("i"), m.get("o")) {
                    if vs.len() > MAX_PATH_VERTICES {
                        return Err(LottieReject::TooComplex("path vertices"));
                    }
                    vertices += vs.len();
                }
                if m.get("ty").and_then(Value::as_str) == Some("sr") {
                    let pt = m.get("pt").and_then(|p| p.get("k")).and_then(Value::as_f64);
                    if pt.is_some_and(|n| !(0.0..=1000.0).contains(&n)) {
                        return Err(LottieReject::TooComplex("star points"));
                    }
                }
                stack.extend(m.values());
            }
            Value::Array(a) => stack.extend(a.iter()),
            _ => {}
        }
        if vertices > MAX_TOTAL_VERTICES {
            return Err(LottieReject::TooComplex("path vertices"));
        }
    }
    // Graph: node 0 = the root composition, then one node per asset that has layers.
    let assets: Vec<&Value> = root.get("assets").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut nodes: Vec<Vec<Value>> = vec![layers_of(&v)];
    for a in &assets {
        if a.get("layers").is_some() {
            if let Some(id) = a.get("id").and_then(Value::as_str) {
                // Two assets with one id: which one a `refId` means is up to the engine, so the
                // graph checked here might not be the graph rlottie walks. Refuse the document.
                if ids.insert(id.to_string(), nodes.len()).is_some() {
                    return Err(LottieReject::Malformed);
                }
                nodes.push(layers_of(a));
            }
        }
    }
    let total: usize = nodes.iter().map(Vec::len).sum();
    if total > 20_000 {
        return Err(LottieReject::TooComplex("layers"));
    }
    // Edges: layer `refId` -> the asset composition it instantiates.
    let edges: Vec<Vec<usize>> = nodes
        .iter()
        .map(|ls| ls.iter().filter_map(|l| l.get("refId").and_then(Value::as_str).and_then(|r| ids.get(r).copied())).collect())
        .collect();
    // Iterative DFS from every node (a cycle among assets the root never reaches is refused too:
    // the engine might still resolve it): cycle detection and depth.
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let mut mark = vec![Mark::New; nodes.len()];
    for start in 0..nodes.len() {
        if mark[start] != Mark::New {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        mark[start] = Mark::Open;
        while let Some(&(n, i)) = stack.last() {
            if stack.len() > 64 {
                return Err(LottieReject::RefCycle);
            }
            if let Some(&next) = edges[n].get(i) {
                stack.last_mut().unwrap().1 += 1;
                match mark[next] {
                    Mark::Open => return Err(LottieReject::RefCycle),
                    Mark::New => {
                        mark[next] = Mark::Open;
                        stack.push((next, 0));
                    }
                    Mark::Done => {}
                }
            } else {
                mark[n] = Mark::Done;
                stack.pop();
            }
        }
    }
    // Expanded layer instances (memoised, saturating): the graph is acyclic now.
    let mut memo: Vec<Option<u64>> = vec![None; nodes.len()];
    let mut order: Vec<usize> = Vec::new();
    let mut seen = vec![false; nodes.len()];
    let mut st = vec![(0usize, 0usize)];
    seen[0] = true;
    while let Some(&(n, i)) = st.last() {
        if let Some(&next) = edges[n].get(i) {
            st.last_mut().unwrap().1 += 1;
            if !seen[next] {
                seen[next] = true;
                st.push((next, 0));
            }
        } else {
            order.push(n);
            st.pop();
        }
    }
    for n in order {
        let mut count = nodes[n].len() as u64;
        for &e in &edges[n] {
            count = count.saturating_add(memo[e].unwrap_or(0));
        }
        memo[n] = Some(count);
    }
    if memo[0].unwrap_or(0) > 200_000 {
        return Err(LottieReject::TooComplex("expanded layers"));
    }
    Ok(())
}

/// Validates ([`validate_lottie`]) and parses a Lottie document.
#[cfg(feature = "native-media")]
pub fn parse_lottie_guarded(data: String) -> Result<rlottie::Animation, LottieReject> {
    validate_lottie(&data)?;
    rlottie::Animation::from_data(data, String::new(), String::new()).ok_or(LottieReject::Engine)
}

/// Parsed Lottie animations by path: a content-only cache (no session state, no hover/click),
/// parsed **outside** any lock that other sessions need, revalidated against the file's
/// `(len, mtime)` (a changed file is reloaded; a rejected file is not re-parsed on every frame).
#[cfg(feature = "native-media")]
pub struct LottieCache {
    entries: Mutex<HashMap<PathBuf, LottieEntry>>,
}

#[cfg(feature = "native-media")]
struct LottieEntry {
    stamp: (u64, Option<std::time::SystemTime>),
    anim: Option<Arc<Mutex<SafeAnimation>>>,
}

#[cfg(feature = "native-media")]
impl LottieCache {
    const MAX_ENTRIES: usize = 64;

    /// Returns the global singleton instance.
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<LottieCache> = OnceLock::new();
        INSTANCE.get_or_init(|| Self { entries: Mutex::new(HashMap::new()) })
    }

    /// The animation of `path`, parsing it on a miss; `None` when missing or rejected (remembered).
    pub fn get_or_load(&self, path: &Path) -> Option<Arc<Mutex<SafeAnimation>>> {
        let meta = std::fs::metadata(path).ok();
        let stamp = meta.as_ref().map_or((0, None), |m| (m.len(), m.modified().ok()));
        {
            let lock = self.entries.lock().unwrap();
            if let Some(e) = lock.get(path) {
                if e.stamp == stamp {
                    return e.anim.clone();
                }
            }
        }
        // Parse without holding the cache lock.
        let anim = match meta {
            Some(m) if m.len() > MAX_LOTTIE_BYTES => {
                tracing::warn!("lottie {:?} refused: {:?}", path, LottieReject::TooLarge);
                None
            }
            _ => crate::render::renderer::Renderer::read_asset(path)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|text| match parse_lottie_guarded(text) {
                    Ok(a) => Some(Arc::new(Mutex::new(SafeAnimation { anim: a }))),
                    Err(e) => {
                        tracing::warn!("lottie {:?} refused: {:?}", path, e);
                        None
                    }
                }),
        };
        let mut lock = self.entries.lock().unwrap();
        if lock.len() >= Self::MAX_ENTRIES && !lock.contains_key(path) {
            lock.clear();
        }
        lock.insert(path.to_path_buf(), LottieEntry { stamp, anim: anim.clone() });
        anim
    }
}

/// A cache for rendered graphic frames.
///
/// Implements a simple FIFO eviction strategy when the capacity limit of 100 is reached.
pub struct AssetCache {
    cache: Mutex<VecDeque<(CacheKey, CacheValue)>>,
}

impl AssetCache {
    const CAPACITY: usize = 100;

    /// Returns the global singleton instance of the cache.
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<AssetCache> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            cache: Mutex::new(VecDeque::new()),
        })
    }

    /// Looks up a cached graphic frame matching the key.
    pub fn lookup(&self, key: &CacheKey) -> Option<CacheValue> {
        let lock = self.cache.lock().unwrap();
        lock.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// Inserts a new graphic frame into the cache, evicting the oldest element if full.
    pub fn insert(&self, key: CacheKey, value: CacheValue) {
        let mut lock = self.cache.lock().unwrap();
        lock.retain(|(k, _)| k != &key);
        if lock.len() >= Self::CAPACITY {
            lock.pop_front();
        }
        lock.push_back((key, value));
    }
}

/// A cache for parsed resvg vector graphics Trees.
///
/// Implements a simple FIFO eviction strategy when the capacity limit of 20 is reached.
#[cfg(feature = "native-media")]
pub struct SvgCache {
    trees: Mutex<VecDeque<(PathBuf, Arc<resvg::usvg::Tree>)>>,
}

#[cfg(feature = "native-media")]
impl SvgCache {
    const CAPACITY: usize = 20;

    /// Returns the global singleton instance of the SVG cache.
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<SvgCache> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            trees: Mutex::new(VecDeque::new()),
        })
    }

    /// Retrieves an SVG tree if cached, otherwise reads the file, parses, and inserts it.
    pub fn get_or_load(&self, path: &Path) -> anyhow::Result<Arc<resvg::usvg::Tree>> {
        let mut lock = self.trees.lock().unwrap();
        if let Some((_, tree)) = lock.iter().find(|(p, _)| p == path) {
            return Ok(Arc::clone(tree));
        }

        let content = crate::render::renderer::Renderer::read_asset(path)?;
        
        let mut fontdb = resvg::usvg::fontdb::Database::new();
        fontdb.load_system_fonts();
        
        let opt = resvg::usvg::Options {
            fontdb: Arc::new(fontdb),
            ..Default::default()
        };
        
        let tree = resvg::usvg::Tree::from_data(&content, &opt)
            .map_err(|e| anyhow::anyhow!("SVG parse error: {:?}", e))?;
        
        let arc_tree = Arc::new(tree);
        if lock.len() >= Self::CAPACITY {
            lock.pop_front();
        }
        lock.push_back((path.to_path_buf(), Arc::clone(&arc_tree)));
        Ok(arc_tree)
    }
}

#[cfg(feature = "native-media")]
use std::process::{Child, Command, Stdio};
#[cfg(feature = "native-media")]
use std::io::Read;
#[cfg(feature = "native-media")]
use std::thread;
#[cfg(feature = "native-media")]
use std::time::Instant;

/// Active video streaming session pipe connecting to an FFmpeg background process.
#[cfg(feature = "native-media")]
pub struct VideoPlayer {
    /// Target frame output width in pixels.
    pub width: u32,
    /// Target frame output height in pixels.
    pub height: u32,
    /// Video playback speed frame-rate (frames per second).
    pub fps: u32,
    /// Thread-shared reference storing the last read raw RGBA frame buffer.
    pub frame_buffer: Arc<RwLock<Option<Arc<Vec<u8>>>>>,
    /// Handle to the child FFmpeg process piping output bytes.
    pub child: Child,
    /// Timestamp of the last successful frame read access.
    pub last_accessed: Instant,
}

#[cfg(feature = "native-media")]
impl Drop for VideoPlayer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Registry managing active background FFmpeg subprocess streams.
#[cfg(feature = "native-media")]
pub struct VideoPlayerRegistry {
    players: Mutex<HashMap<(PathBuf, u32, u32, u32), VideoPlayer>>,
}

#[cfg(feature = "native-media")]
impl VideoPlayerRegistry {
    /// Returns the global singleton instance of the registry.
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<VideoPlayerRegistry> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            players: Mutex::new(HashMap::new()),
        })
    }

    /// Checks if the `ffmpeg` executable is installed and available in system environment path.
    pub fn is_ffmpeg_available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            Command::new("ffmpeg")
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        })
    }

    /// Reads the current frame buffer from a video playback stream.
    ///
    /// Spawns a new FFmpeg process if no stream is currently active for this file configuration.
    /// Cleans up stale inactive streams.
    pub fn get_frame(&self, path: &Path, width: u32, height: u32, fps: u32) -> Option<Arc<Vec<u8>>> {
        let mut players = self.players.lock().unwrap();
        let now = Instant::now();

        // 1. Clean up stale players (inactivity > 5 seconds)
        players.retain(|_, player| {
            now.duration_since(player.last_accessed) <= std::time::Duration::from_secs(5)
        });

        // 2. Lookup or create player
        let key = (path.to_path_buf(), width, height, fps);
        if !players.contains_key(&key) {
            if let Some(player) = Self::spawn_player(path, width, height, fps) {
                players.insert(key.clone(), player);
            } else {
                return None;
            }
        }

        if let Some(player) = players.get_mut(&key) {
            player.last_accessed = now;
            player.frame_buffer.read().unwrap().clone()
        } else {
            None
        }
    }

    fn spawn_player(path: &Path, width: u32, height: u32, fps: u32) -> Option<VideoPlayer> {
        let child = Command::new("ffmpeg")
            .args(&[
                "-stream_loop", "-1", // Loop input infinitely.
                "-i", &path.to_string_lossy(),
                "-r", &fps.to_string(),
                "-vf", &format!("scale={}:{}", width, height),
                "-f", "image2pipe",
                "-pix_fmt", "rgba",
                "-vcodec", "rawvideo",
                "-"
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(mut child) => {
                let mut stdout = child.stdout.take().unwrap();
                let frame_buffer: Arc<RwLock<Option<Arc<Vec<u8>>>>> = Arc::new(RwLock::new(None));
                let frame_buffer_clone = Arc::clone(&frame_buffer);
                let frame_size = (width * height * 4) as usize;

                thread::spawn(move || {
                    loop {
                        let mut buf = vec![0u8; frame_size];
                        if stdout.read_exact(&mut buf).is_err() {
                            break;
                        }
                        *frame_buffer_clone.write().unwrap() = Some(Arc::new(buf));
                    }
                });

                Some(VideoPlayer {
                    width,
                    height,
                    fps,
                    frame_buffer,
                    child,
                    last_accessed: Instant::now(),
                })
            }
            Err(e) => {
                tracing::warn!("Failed to spawn ffmpeg for video {:?}: {:?}", path, e);
                None
            }
        }
    }

    /// Kills and cleans up all active video streaming sub-processes.
    pub fn cleanup(&self) {
        let mut players = self.players.lock().unwrap();
        players.clear();
    }
}

#[cfg(all(test, feature = "native-media"))]
mod lottie_guard_tests {
    use super::*;

    fn doc(assets: &str, layers: &str) -> String {
        format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{assets}],"layers":[{layers}]}}"#)
    }
    const REF_A: &str = r#"{"ty":0,"refId":"a","ind":1}"#;

    #[test]
    fn plain_documents_pass() {
        assert_eq!(validate_lottie(&doc("", "")), Ok(()));
        assert_eq!(validate_lottie(&doc(r#"{"id":"a","layers":[{"ty":4}]}"#, REF_A)), Ok(()));
    }

    #[test]
    fn precomp_cycles_are_rejected() {
        let selfref = r#"{"id":"a","layers":[{"ty":0,"refId":"a"}]}"#;
        assert_eq!(validate_lottie(&doc(selfref, REF_A)), Err(LottieReject::RefCycle));
        let two = r#"{"id":"a","layers":[{"ty":0,"refId":"b"}]},{"id":"b","layers":[{"ty":0,"refId":"a"}]}"#;
        assert_eq!(validate_lottie(&doc(two, REF_A)), Err(LottieReject::RefCycle));
    }

    #[test]
    fn unreachable_cycles_and_duplicate_ids_are_rejected() {
        let lone = r#"{"id":"z","layers":[{"ty":0,"refId":"z"}]}"#;
        assert_eq!(validate_lottie(&doc(lone, "")), Err(LottieReject::RefCycle), "a cycle the root never reaches");
        let dup = r#"{"id":"a","layers":[{"ty":0,"refId":"a"}]},{"id":"a","layers":[]}"#;
        assert_eq!(validate_lottie(&doc(dup, REF_A)), Err(LottieReject::Malformed), "duplicate asset id");
        // a 3000-deep chain is refused by the depth cap (reported as RefCycle)
        let chain: Vec<String> = (0..3000).map(|i| format!(r#"{{"id":"c{i}","layers":[{{"refId":"c{n}"}}]}}"#, n = i + 1)).collect();
        assert_eq!(validate_lottie(&doc(&chain.join(","), r#"{"refId":"c0"}"#)), Err(LottieReject::RefCycle));
    }

    #[test]
    fn exponential_and_oversized_documents_are_rejected() {
        let assets: Vec<String> = (0..30).map(|i| format!(r#"{{"id":"c{i}","layers":[{{"refId":"c{n}"}},{{"refId":"c{n}"}}]}}"#, n = i + 1)).chain([r#"{"id":"c30","layers":[{"ty":4}]}"#.to_string()]).collect();
        // 30 levels would also be deeper than the 64-level cap only at 65+: this one is the instance count.
        assert!(matches!(validate_lottie(&doc(&assets.join(","), r#"{"refId":"c0"}"#)), Err(LottieReject::TooComplex(_))));
        assert_eq!(validate_lottie(&"[".repeat(100_000)), Err(LottieReject::Malformed));
        assert_eq!(validate_lottie(&" ".repeat(MAX_LOTTIE_BYTES as usize + 1)), Err(LottieReject::TooLarge));
        assert!(matches!(validate_lottie(r#"{"fr":30,"ip":0,"op":60,"w":2000000000,"h":10,"layers":[]}"#), Err(LottieReject::TooComplex("size"))));
        let pts = vec!["[1,1]"; MAX_PATH_VERTICES + 1].join(",");
        let big = doc("", &format!(r#"{{"shapes":[{{"ks":{{"k":{{"v":[{pts}],"i":[],"o":[]}}}}}}]}}"#));
        assert!(matches!(validate_lottie(&big), Err(LottieReject::TooComplex("path vertices"))));
    }
}
