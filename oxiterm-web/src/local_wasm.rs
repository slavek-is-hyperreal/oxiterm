//! wasm-bindgen face of the local mode (OxiUI 7.1): a thin wrapper over
//! [`crate::local::LocalSession`]. The page's JS owns timers, `fetch` and the DOM; this owns the
//! core, the painter call and the embedded fonts.

use wasm_bindgen::prelude::*;

use crate::local::{LocalConfig, LocalSession};
use crate::WebTerminal;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(s: &str);
}

/// Routes Rust panics to `console.error` (otherwise a wasm panic is a bare "unreachable").
fn install_panic_hook() {
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| std::panic::set_hook(Box::new(|info| console_error(&format!("oxiterm-web panic: {info}")))));
    }
}

/// The core running in the browser.
#[wasm_bindgen]
pub struct LocalApp {
    s: LocalSession,
}

#[wasm_bindgen]
impl LocalApp {
    /// `pages_json` is the server's page index (`{"pages": {"path": "THTML text", ...}, "dirs": [...]}`); times are
    /// `performance.now()` milliseconds.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pages_json: &str,
        start_page: &str,
        cols: u16,
        rows: u16,
        mobile: bool,
        actions: bool,
        app_token: &str,
        now_ms: f64,
    ) -> Result<LocalApp, String> {
        install_panic_hook();
        let v: serde_json::Value = serde_json::from_str(pages_json).map_err(|e| format!("pages: {e}"))?;
        let pages: Vec<(String, String)> = v["pages"]
            .as_object()
            .ok_or("pages: no `pages` object")?
            .iter()
            .filter_map(|(k, t)| t.as_str().map(|t| (k.clone(), t.to_string())))
            .collect();
        let dirs: Vec<String> = v["dirs"].as_array().map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let cfg = LocalConfig { start_page: start_page.to_string(), cols, rows, mobile, actions, app_token: app_token.to_string() };
        Ok(LocalApp { s: LocalSession::new_with_dirs(&cfg, pages, dirs, now_ms)? })
    }

    /// Feeds one web-protocol input message (0x01/0x02/0x03/0x10/0x11/0x31).
    pub fn input(&mut self, bytes: &[u8], now_ms: f64) {
        self.s.input(bytes, now_ms);
    }

    /// Runs the loop (reply queue, tick) and paints the newest frame, if any, straight from
    /// the display list: no wire encode, no inflate.
    pub fn pump(&mut self, term: &mut WebTerminal, now_ms: f64) {
        self.s.pump(now_ms);
        if let Some(list) = self.s.take_frames().pop() {
            term.paint_list(&list);
        }
    }

    /// Repaints the last list (after a canvas resize that cleared the canvas).
    pub fn repaint(&mut self, term: &mut WebTerminal) {
        if let Some(list) = self.s.last_list() {
            term.paint_list(&list);
        }
    }

    /// Milliseconds until `pump` is next due.
    pub fn next_delay_ms(&self, now_ms: f64) -> f64 {
        self.s.next_delay_ms(now_ms)
    }

    /// Side-channel messages since the last call: `[u32le len][bytes]` repeated. Same tags as
    /// the WebSocket protocol (0x30, 0x33..0x35, 0x37, 0x21) plus local-only 0xE0.
    pub fn take_messages(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for m in self.s.take_messages() {
            out.extend_from_slice(&(m.len() as u32).to_le_bytes());
            out.extend_from_slice(&m);
        }
        out
    }

    /// Actions to forward to the proxy: `[{"id":n,"body":"<json payload>"}]`.
    pub fn take_actions(&mut self) -> String {
        let list: Vec<serde_json::Value> =
            self.s.take_actions().into_iter().map(|a| serde_json::json!({ "id": a.id, "body": a.body })).collect();
        serde_json::Value::Array(list).to_string()
    }

    /// The App Server answered action `id` with `json`. False when the text is not JSON.
    pub fn action_reply(&mut self, id: u32, json: &str) -> bool {
        match serde_json::from_str::<serde_json::Value>(json) {
            Ok(v) => {
                self.s.action_reply(id, v);
                true
            }
            Err(_) => {
                self.s.action_failed(id);
                false
            }
        }
    }

    /// Action `id` failed (network, non-200): no response, like a dead App Server.
    pub fn action_failed(&mut self, id: u32) {
        self.s.action_failed(id);
    }

    /// Re-emits media coordinates after the page cached a requested asset.
    pub fn media_refresh(&mut self) {
        self.s.media_refresh();
    }
}

/// `FontBundle::metrics()` as JSON: the same document `/oxiui/fonts.json` serves.
#[wasm_bindgen]
pub fn local_fonts_json() -> String {
    serde_json::to_string(oxiterm_renderer::text::bundle::FontBundle::metrics()).unwrap_or_default()
}

/// The bytes of an embedded face (`oxi-mono`, `oxi-sans`) for `new FontFace(id, bytes)`: the
/// core already carries them, so local mode never downloads them a second time.
#[wasm_bindgen]
pub fn local_font_bytes(id: &str) -> Vec<u8> {
    oxiterm_renderer::text::bundle::FontBundle::font_bytes(id).map(<[u8]>::to_vec).unwrap_or_default()
}
