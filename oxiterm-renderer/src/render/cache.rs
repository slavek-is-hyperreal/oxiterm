use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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

pub struct PlaybackState {
    pub start_time: std::time::Instant,
    pub hover: bool,
    pub click_active: bool,
    pub click_coord: Option<(u16, u16)>,
    pub toggled: bool,
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
            return PlaybackState {
                start_time: state.start_time,
                hover: state.hover,
                click_active: state.click_active,
                click_coord: state.click_coord,
                toggled: state.toggled,
            };
        }
        let state = PlaybackState {
            start_time: std::time::Instant::now(),
            hover: false,
            click_active: false,
            click_coord: None,
            toggled: false,
        };
        lock.insert(path.to_path_buf(), PlaybackState {
            start_time: state.start_time,
            hover: state.hover,
            click_active: state.click_active,
            click_coord: state.click_coord,
            toggled: state.toggled,
        });
        state
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

pub struct AssetCache {
    cache: Mutex<HashMap<CacheKey, CacheValue>>,
}

impl AssetCache {
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<AssetCache> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn lookup(&self, key: &CacheKey) -> Option<CacheValue> {
        let lock = self.cache.lock().unwrap();
        lock.get(key).cloned()
    }

    pub fn insert(&self, key: CacheKey, value: CacheValue) {
        let mut lock = self.cache.lock().unwrap();
        lock.insert(key, value);
    }
}

pub struct SvgCache {
    trees: Mutex<HashMap<PathBuf, Arc<resvg::usvg::Tree>>>,
}

impl SvgCache {
    pub fn get() -> &'static Self {
        static INSTANCE: OnceLock<SvgCache> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            trees: Mutex::new(HashMap::new()),
        })
    }

    pub fn get_or_load(&self, path: &Path) -> anyhow::Result<Arc<resvg::usvg::Tree>> {
        let mut lock = self.trees.lock().unwrap();
        if let Some(tree) = lock.get(path) {
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
        lock.insert(path.to_path_buf(), Arc::clone(&arc_tree));
        Ok(arc_tree)
    }
}
