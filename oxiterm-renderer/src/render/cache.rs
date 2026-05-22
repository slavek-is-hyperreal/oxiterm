use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, OnceLock};
use std::process::{Command, Stdio, Child};
use std::io::Read;
use std::thread;
use std::time::Instant;

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct CacheKey {
    pub path: PathBuf,
    pub width_px: u32,
    pub height_px: u32,
    pub frame_idx: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum GraphicFormat {
    Sixel(Vec<u8>),
    Kitty(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct CacheValue {
    pub format: GraphicFormat,
}

pub struct SafeAnimation {
    pub anim: rlottie::Animation,
}
unsafe impl Send for SafeAnimation {}
// NOTE: Sync is intentionally NOT implemented — SafeAnimation wraps a C FFI type
// (rlottie::Animation) that relies on thread-local state. Access is always through
// Arc<Mutex<SafeAnimation>>, which guarantees exclusive access per thread.

#[derive(Clone)]
pub struct PlaybackState {
    pub start_time: std::time::Instant,
    pub hover: bool,
    pub click_active: bool,
    pub click_coord: Option<(u16, u16)>,
    pub toggled: bool,
    pub lottie_animation: Option<Arc<Mutex<SafeAnimation>>>,
}

pub struct PlaybackRegistry {
    states: Mutex<HashMap<PathBuf, PlaybackState>>,
}

impl PlaybackRegistry {
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<PlaybackRegistry> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            states: Mutex::new(HashMap::new()),
        })
    }

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
            lottie_animation: None,
        };
        lock.insert(path.to_path_buf(), state.clone());
        state
    }

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
            if let Ok(data_str) = std::fs::read_to_string(path) {
                if let Some(anim) = rlottie::Animation::from_data(data_str, String::new(), String::new()) {
                    state.lottie_animation = Some(Arc::new(Mutex::new(SafeAnimation { anim })));
                }
            }
        }
        state.lottie_animation.clone()
    }

    pub fn set_hover(&self, path: &Path, hover: bool) {
        let mut lock = self.states.lock().unwrap();
        if let Some(state) = lock.get_mut(path) {
            state.hover = hover;
        }
    }

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

/// FIFO-eviction asset cache. Insertion order is preserved via VecDeque;
/// oldest entry is evicted when capacity is reached (capacity = 100).
pub struct AssetCache {
    cache: Mutex<VecDeque<(CacheKey, CacheValue)>>,
}

impl AssetCache {
    const CAPACITY: usize = 100;

    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<AssetCache> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            cache: Mutex::new(VecDeque::new()),
        })
    }

    pub fn lookup(&self, key: &CacheKey) -> Option<CacheValue> {
        let lock = self.cache.lock().unwrap();
        lock.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    pub fn insert(&self, key: CacheKey, value: CacheValue) {
        let mut lock = self.cache.lock().unwrap();
        // Remove existing entry with the same key (update in place would keep order, but
        // re-inserting at the back is simpler and semantically correct for an asset cache).
        lock.retain(|(k, _)| k != &key);
        if lock.len() >= Self::CAPACITY {
            lock.pop_front(); // FIFO eviction — remove the oldest entry
        }
        lock.push_back((key, value));
    }
}

/// FIFO-eviction SVG tree cache (capacity = 20).
pub struct SvgCache {
    trees: Mutex<VecDeque<(PathBuf, Arc<resvg::usvg::Tree>)>>,
}

impl SvgCache {
    const CAPACITY: usize = 20;

    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<SvgCache> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            trees: Mutex::new(VecDeque::new()),
        })
    }

    pub fn get_or_load(&self, path: &Path) -> anyhow::Result<Arc<resvg::usvg::Tree>> {
        let mut lock = self.trees.lock().unwrap();
        if let Some((_, tree)) = lock.iter().find(|(p, _)| p == path) {
            return Ok(Arc::clone(tree));
        }

        let content = std::fs::read(path)?;
        
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
            lock.pop_front(); // FIFO eviction
        }
        lock.push_back((path.to_path_buf(), Arc::clone(&arc_tree)));
        Ok(arc_tree)
    }
}

pub struct VideoPlayer {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// RwLock instead of Mutex: the ffmpeg thread writes once per decoded frame,
    /// while the renderer thread reads (potentially) many times per render cycle.
    pub frame_buffer: Arc<RwLock<Option<Arc<Vec<u8>>>>>,
    pub child: Child,
    pub last_accessed: Instant,
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}


pub struct VideoPlayerRegistry {
    players: Mutex<HashMap<(PathBuf, u32, u32, u32), VideoPlayer>>,
}

impl VideoPlayerRegistry {
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<VideoPlayerRegistry> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            players: Mutex::new(HashMap::new()),
        })
    }

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

    pub fn get_frame(&self, path: &Path, width: u32, height: u32, fps: u32) -> Option<Arc<Vec<u8>>> {
        let mut players = self.players.lock().unwrap();
        let now = Instant::now();

        // 1. Clean up stale players (inactivity > 5 seconds)
        players.retain(|_, player| {
            if now.duration_since(player.last_accessed) > std::time::Duration::from_secs(5) {
                false
            } else {
                true
            }
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
                "-stream_loop", "-1", // loop input infinitely
                "-i", &path.to_string_lossy(),
                "-r", &fps.to_string(), // output frame rate
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

    pub fn cleanup(&self) {
        let mut players = self.players.lock().unwrap();
        players.clear();
    }
}
