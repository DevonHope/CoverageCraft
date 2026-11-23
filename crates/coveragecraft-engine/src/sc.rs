//! SolidCraft designs (`.sc`): SolidCraft itself turns a design into a mesh, so CoverageCraft never
//! reads SolidCraft's kernel format. Its command-line tool (`solidcraft-cli`) runs the design's
//! history and exports every body as an OBJ object, and lists the views saved in the design.
//!
//! - The tool is found at `COVERAGECRAFT_SOLIDCRAFT_CLI`, then on `PATH`, then inside
//!   `SolidCraft.app` or `SolidCraft Dev.app` in `/Applications` or `~/Applications` (macOS).
//! - It runs without a shell, with a time limit, in a temporary folder that is removed after, and
//!   with SolidCraft's settings, recovery and control folders pointed there, so opening a design
//!   here leaves the user's SolidCraft (its recent files, its settings) as it was.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::viewpoint::{Basis, PlanView};

/// How long SolidCraft may take to rebuild and export a design.
const TIME_LIMIT: Duration = Duration::from_secs(300);
/// The error a cancelled export ends with.
pub const CANCELLED: &str = "cancelled";
/// The largest OBJ read back (bytes).
const MAX_OBJ_BYTES: u64 = 1 << 30;

/// A design as SolidCraft exported it.
#[derive(Debug)]
pub struct Converted {
    pub obj: Vec<u8>,
    /// The views saved in the design, as viewpoints (Z-up ones; others are noted and left out).
    pub views: Vec<PlanView>,
    pub notes: Vec<String>,
}

/// Where `solidcraft-cli` is, if anywhere.
pub fn find_cli() -> Option<PathBuf> {
    // RADIOCRAFT_SOLIDCRAFT_CLI: the name before the app was renamed.
    if let Some(p) = std::env::var_os("COVERAGECRAFT_SOLIDCRAFT_CLI")
        .or_else(|| std::env::var_os("RADIOCRAFT_SOLIDCRAFT_CLI"))
        .map(PathBuf::from)
    {
        return p.is_file().then_some(p);
    }
    let name = if cfg!(windows) {
        "solidcraft-cli.exe"
    } else {
        "solidcraft-cli"
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if cfg!(target_os = "macos") {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        for root in roots {
            for app in ["SolidCraft.app", "SolidCraft Dev.app"] {
                let p = root.join(app).join("Contents/MacOS/solidcraft-cli");
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// A fresh, empty temporary folder of our own.
fn scratch() -> Result<PathBuf, String> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "coveragecraft-sc-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("a temporary folder: {e}"))?;
    Ok(dir)
}

/// Export the design at `path` with SolidCraft.
pub fn convert(path: &Path) -> Result<Converted, String> {
    convert_with(path, &AtomicBool::new(false))
}

/// As [`convert`], stopping SolidCraft as soon as `cancel` is set (from another thread): a
/// window can run the export in the background and offer to cancel it.
pub fn convert_with(path: &Path, cancel: &AtomicBool) -> Result<Converted, String> {
    let cli = find_cli().ok_or(
        "opening a SolidCraft design needs SolidCraft installed (its solidcraft-cli), or \
         COVERAGECRAFT_SOLIDCRAFT_CLI set to it; or export the design from SolidCraft as OBJ",
    )?;
    let dir = scratch()?;
    let result = run(&cli, path, &dir, cancel);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run(cli: &Path, design: &Path, dir: &Path, cancel: &AtomicBool) -> Result<Converted, String> {
    let obj = dir.join("design.obj");
    let script = dir.join("export.json");
    let commands = json!({"commands": [
        {"command": "view.list"},
        {"command": "file.export_obj", "params": {"path": obj.to_string_lossy()}},
    ]});
    std::fs::write(&script, commands.to_string()).map_err(|e| e.to_string())?;
    let out = dir.join("results.json");
    // To a file, not a pipe: a full pipe would stall the export while we wait for it.
    let err_path = dir.join("stderr.txt");
    let err_file = std::fs::File::create(&err_path).map_err(|e| e.to_string())?;
    let mut child = Command::new(cli)
        .arg("run")
        .arg(&script)
        .arg("--in")
        .arg(design)
        .arg("--results")
        .arg("--quiet")
        .arg("--json-out")
        .arg(&out)
        .current_dir(dir)
        .env("SOLIDCRAFT_SETTINGS_DIR", dir.join("settings"))
        .env("SOLIDCRAFT_RECOVERY_DIR", dir.join("recovery"))
        .env("SOLIDCRAFT_CONTROL_DIR", dir.join("control"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(|e| format!("{}: {e}", cli.display()))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if cancel.load(Ordering::Relaxed) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CANCELLED.into());
            }
            Ok(None) if started.elapsed() > TIME_LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "SolidCraft took longer than {} s to export the design",
                    TIME_LIMIT.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let mut stderr = String::new();
    if let Ok(f) = std::fs::File::open(&err_path) {
        use std::io::Read;
        let _ = f.take(64 << 10).read_to_string(&mut stderr);
    }
    // SolidCraft's warnings about the design, without the folder it was read from.
    let notes: Vec<String> = stderr
        .lines()
        .filter_map(|l| {
            l.split_once("warning: ")
                .map(|(_, w)| format!("SolidCraft: {w}"))
        })
        .take(10)
        .collect();
    if !status.success() {
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no message");
        let last = last.rsplit_once(": ").map_or(last, |(_, m)| m);
        return Err(format!("SolidCraft couldn't export the design: {last}"));
    }
    let meta = std::fs::metadata(&obj).map_err(|_| "SolidCraft wrote no mesh".to_string())?;
    if meta.len() > MAX_OBJ_BYTES {
        return Err("the design's mesh is larger than 1 GB".into());
    }
    let obj_bytes = std::fs::read(&obj).map_err(|e| e.to_string())?;
    let results: Value = std::fs::read(&out)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let (views, mut more) = saved_views(&results);
    let mut notes = notes;
    notes.append(&mut more);
    Ok(Converted {
        obj: obj_bytes,
        views,
        notes,
    })
}

/// The views `view.list` returned, as viewpoints: Z-up ones only.
fn saved_views(results: &Value) -> (Vec<PlanView>, Vec<String>) {
    let list = results
        .as_array()
        .and_then(|r| r.iter().find_map(|x| x.get("views")))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut views = Vec::new();
    let mut notes = Vec::new();
    for v in list.iter().take(200) {
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if name.is_empty() {
            continue;
        }
        if v.get("up")
            .and_then(Value::as_str)
            .is_some_and(|u| u != "z")
        {
            notes.push(format!(
                "the saved view \"{name}\" is in a Y-up frame, which CoverageCraft doesn't read yet"
            ));
            continue;
        }
        let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        if let Some(basis) = Basis::from_camera(f("yaw"), f("pitch"), f("roll")) {
            views.push(PlanView {
                id: format!("view:{name}"),
                name: name.to_string(),
                basis: Some(basis),
                window: None,
            });
        }
    }
    (views, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_views_are_read_from_the_results() {
        let results = json!([
            {"view": "x"},
            {"views": [
                {"name": "Plan", "yaw": 0.0, "pitch": std::f64::consts::FRAC_PI_2, "roll": 0.0, "up": "z"},
                {"name": "Sideways", "yaw": 0.0, "pitch": 0.0, "up": "y"},
                {"name": "", "yaw": 0.0, "pitch": 0.0},
            ]},
        ]);
        let (views, notes) = saved_views(&results);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].id, "view:Plan");
        assert_eq!(notes.len(), 1);
        assert!(saved_views(&json!(null)).0.is_empty());
    }
}
