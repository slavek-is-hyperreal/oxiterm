//! `oxi new <name> [--template ...]`: write a project that passes `oxi check`.
//! Templates are embedded in the binary, so an installed `oxi` works anywhere.

use crate::check::{EXIT_FATAL, EXIT_OK};
use std::path::{Path, PathBuf};

pub const TEMPLATES: &[&str] = &["blink", "touch-button", "sensor-dashboard", "oxiui-page"];

type Files = &'static [(&'static str, &'static str)];

const BLINK: Files = &[
    ("main.oxs", include_str!("../templates/blink/main.oxs")),
    ("oxi.toml", include_str!("../templates/blink/oxi.toml")),
    ("idl/gpio.idl", include_str!("../templates/blink/idl/gpio.idl")),
];
const TOUCH_BUTTON: Files = &[
    ("main.oxs", include_str!("../templates/touch-button/main.oxs")),
    ("oxi.toml", include_str!("../templates/touch-button/oxi.toml")),
    ("idl/ui.idl", include_str!("../templates/touch-button/idl/ui.idl")),
];
const SENSOR_DASHBOARD: Files = &[
    ("main.oxs", include_str!("../templates/sensor-dashboard/main.oxs")),
    ("oxi.toml", include_str!("../templates/sensor-dashboard/oxi.toml")),
    ("idl/sensor.idl", include_str!("../templates/sensor-dashboard/idl/sensor.idl")),
];
const OXIUI_PAGE: Files = &[
    ("main.oxs", include_str!("../templates/oxiui-page/main.oxs")),
    ("oxi.toml", include_str!("../templates/oxiui-page/oxi.toml")),
    ("idl/ui.idl", include_str!("../templates/oxiui-page/idl/ui.idl")),
];
const COMMON: Files = &[
    ("AGENTS.md", include_str!("../templates/common/AGENTS.md")),
    (".vscode/extensions.json", include_str!("../templates/common/vscode-extensions.json")),
    (".vscode/settings.json", include_str!("../templates/common/vscode-settings.json")),
    (".gitignore", "*.wasm\n"),
];

fn template(name: &str) -> Option<Files> {
    Some(match name {
        "blink" => BLINK,
        "touch-button" => TOUCH_BUTTON,
        "sensor-dashboard" => SENSOR_DASHBOARD,
        "oxiui-page" => OXIUI_PAGE,
        _ => return None,
    })
}

pub fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    n.len() <= 64
        && c.next().is_some_and(|f| f.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '-' || x == '_')
}

/// Write the files of `template` into `dir` (which must not exist).
pub fn create(dir: &Path, name: &str, tpl: &str) -> Result<Vec<PathBuf>, String> {
    let files = template(tpl).ok_or_else(|| format!("unknown template `{tpl}` (choose one of: {})", TEMPLATES.join(", ")))?;
    if !valid_name(name) {
        return Err(format!("`{name}` is not a valid project name (lowercase letters, digits, `-` and `_`, starting with a letter)"));
    }
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let mut written = Vec::new();
    for (rel, text) in COMMON.iter().chain(files.iter()) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&p, text.replace("{{name}}", name)).map_err(|e| format!("{}: {e}", p.display()))?;
        written.push(p);
    }
    Ok(written)
}

pub fn run(name: &str, tpl: &str) -> i32 {
    let dir = PathBuf::from(name);
    match create(&dir, name, tpl) {
        Ok(files) => {
            for f in &files {
                println!("created {}", f.display());
            }
            println!("\nNext:\n  cd {name}\n  oxi check\n  code .");
            EXIT_OK
        }
        Err(e) => {
            eprintln!("error: {e}");
            EXIT_FATAL
        }
    }
}
