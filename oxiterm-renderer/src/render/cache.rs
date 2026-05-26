//! Cache registries for media playback and graphics generation.
//!
//! Provides FIFO-eviction caches for images, SVG trees, Lottie animations, and
//! processes video rendering streams via external tools such as FFmpeg.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, OnceLock};

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
#[cfg(not(target_arch = "wasm32"))]
pub struct SafeAnimation {
    /// The underlying rlottie Animation instance.
    pub anim: rlottie::Animation,
}
#[cfg(not(target_arch = "wasm32"))]
unsafe impl Send for SafeAnimation {}

/// Playback tracking state of an animated or interactive media asset.
#[derive(Clone)]
pub struct PlaybackState {
    /// Timestamp when media rendering started.
    pub start_time: std::time::Instant,
    /// Is the user cursor currently hovering over this element.
    pub hover: bool,
    /// Is the element currently clicked or pressed.
    pub click_active: bool,
    /// Coordinates of the last click (column, row).
    pub click_coord: Option<(u16, u16)>,
    /// State toggle switcher state (e.g. play/pause).
    pub toggled: bool,
    /// Optional reference to the underlying Lottie animation.
    #[cfg(not(target_arch = "wasm32"))]
    pub lottie_animation: Option<Arc<Mutex<SafeAnimation>>>,
}

/// Registry storing state for all playing media instances.
pub struct PlaybackRegistry {
    states: Mutex<HashMap<PathBuf, PlaybackState>>,
}

impl PlaybackRegistry {
    /// Returns the global thread-safe singleton instance of the registry.
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<PlaybackRegistry> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            states: Mutex::new(HashMap::new()),
        })
    }

    /// Fetches the playback state of the given path, creating a new default one if missing.
    pub fn get_or_create(&self, path: &Path) -> PlaybackState {
        let mut lock = self.states.lock().unwrap();
        if let Some(state) = lock.get(path) {
            return state.clone();
        }
        let state = PlaybackState {
            start_time: std::time::Instant::now(),
            hover: false,
            click_active: false,
            click_coord: None,
            toggled: false,
            #[cfg(not(target_arch = "wasm32"))]
            lottie_animation: None,
        };
        lock.insert(path.to_path_buf(), state.clone());
        state
    }

    /// Fetches or parses Lottie animation files into the playback registry.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn get_or_load_lottie(&self, path: &Path) -> Option<Arc<Mutex<SafeAnimation>>> {
        let mut lock = self.states.lock().unwrap();
        let state = lock.entry(path.to_path_buf()).or_insert_with(|| PlaybackState {
            start_time: std::time::Instant::now(),
            hover: false,
            click_active: false,
            click_coord: None,
            toggled: false,
            lottie_animation: None,
        });

        if state.lottie_animation.is_none() {
            if let Ok(bytes) = crate::render::renderer::Renderer::read_asset(path) {
                if let Ok(data_str) = String::from_utf8(bytes) {
                    if let Some(anim) = rlottie::Animation::from_data(data_str, String::new(), String::new()) {
                        state.lottie_animation = Some(Arc::new(Mutex::new(SafeAnimation { anim })));
                    }
                }
            }
        }
        state.lottie_animation.clone()
    }

    /// Sets the hover flag of a registered playback state.
    pub fn set_hover(&self, path: &Path, hover: bool) {
        let mut lock = self.states.lock().unwrap();
        if let Some(state) = lock.get_mut(path) {
            state.hover = hover;
        }
    }

    /// Sets the click state parameters of a registered playback instance.
    pub fn set_click(&self, path: &Path, active: bool, coord: Option<(u16, u16)>) {
        let mut lock = self.states.lock().unwrap();
        if let Some(state) = lock.get_mut(path) {
            state.click_active = active;
            state.click_coord = coord;
            if active {
                state.toggled = !state.toggled;
            }
        }
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
#[cfg(not(target_arch = "wasm32"))]
pub struct SvgCache {
    trees: Mutex<VecDeque<(PathBuf, Arc<resvg::usvg::Tree>)>>,
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, Command, Stdio};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Active video streaming session pipe connecting to an FFmpeg background process.
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
impl Drop for VideoPlayer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Registry managing active background FFmpeg subprocess streams.
#[cfg(not(target_arch = "wasm32"))]
pub struct VideoPlayerRegistry {
    players: Mutex<HashMap<(PathBuf, u32, u32, u32), VideoPlayer>>,
}

#[cfg(not(target_arch = "wasm32"))]
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
