//! Taught drawings on disk, one `<name>.json` per drawing in
//! `$XDG_DATA_HOME/air-gestures/drawings` (else `~/.local/share/air-gestures/drawings`):
//! `{"version": 1, "tries": [[[[x, y], …], …], …]}`, each try a list of strokes, each stroke
//! a list of [x, y] points in degrees (band-core's `draw::Stroke`).

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use band_core::controls::draw::{Drawing, Stroke};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    tries: Vec<Vec<Stroke>>,
}

/// The directory (the dry-run daemon's is separate, like its state).
pub fn default_dir(dry_run: bool) -> PathBuf {
    let base = crate::config::xdg("XDG_DATA_HOME").unwrap_or_else(|| crate::config::home().join(".local/share"));
    base.join(if dry_run { "air-gestures-dry-run" } else { "air-gestures" }).join("drawings")
}

/// Drawing names are file names and go into `AIR_GESTURES_DRAWING='…'`.
pub fn check_name(name: &str) -> Result<(), String> {
    crate::config::check_name("drawing", name)
}

fn path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

fn invalid(error: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}

/// Every drawing in `dir`, by name; what couldn't be read, described. No directory: none.
pub fn load(dir: &Path) -> (Vec<Drawing>, Vec<String>) {
    let (mut drawings, mut problems) = (Vec::new(), Vec::new());
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (drawings, problems),
        Err(error) => {
            problems.push(format!("{}: {error}", dir.display()));
            return (drawings, problems);
        }
    };
    for entry in entries.flatten() {
        let file = entry.path();
        let Some(name) = file.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".json")) else { continue };
        if check_name(name).is_err() {
            continue;
        }
        let read = std::fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<File>(&bytes).map_err(|e| e.to_string()));
        match read {
            Ok(f) if f.version == VERSION => drawings.push(Drawing { name: name.to_owned(), tries: f.tries }),
            Ok(f) => problems.push(format!("{}: version {} (this air-gesturesd reads {VERSION})", file.display(), f.version)),
            Err(error) => problems.push(format!("{}: {error}", file.display())),
        }
    }
    drawings.sort_by(|a, b| a.name.cmp(&b.name));
    (drawings, problems)
}

/// Write it atomically (a temp file renamed into place), creating `dir` if needed.
pub fn save(dir: &Path, drawing: &Drawing) -> io::Result<()> {
    check_name(&drawing.name).map_err(invalid)?;
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(&File { version: VERSION, tries: drawing.tries.clone() }).map_err(io::Error::other)?;
    let temp = dir.join(format!(".{}.json.tmp", drawing.name));
    std::fs::write(&temp, json)?;
    std::fs::rename(&temp, path(dir, &drawing.name))
}

/// Remove it; `false` when there was none.
pub fn delete(dir: &Path, name: &str) -> io::Result<bool> {
    check_name(name).map_err(invalid)?;
    match std::fs::remove_file(path(dir, name)) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Changes whenever a drawing is saved or deleted: the directory's modification time.
pub fn stamp(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}
