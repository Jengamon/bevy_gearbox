use bevy::prelude::*;
use bevy::scene::{DynamicScene, DynamicSceneRoot};

/// Spawn a loader entity to load a Bevy DynamicScene from an asset path.
/// The asset path should be AssetServer-relative (e.g., "app_state.scn.ron").
pub fn load_graph_from_file(commands: &mut Commands, asset_server: &AssetServer, file_path: impl Into<String>) -> Entity {
    let path: String = file_path.into();
    let handle: Handle<DynamicScene> = asset_server.load(path);
    commands
        .spawn((Name::new("State Machine (scene)"), DynamicSceneRoot(handle)))
        .id()
}

// ============
// Sidecar (.sm.ron) save
// ============
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let tmp_path: PathBuf = {
        let mut p = path.to_path_buf();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            p.set_file_name(format!("{}.tmp", name));
        } else {
            p.set_file_name("sidecar.tmp");
        }
        p
    };

    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(contents.as_bytes())?;
        f.flush()?;
    }
    #[cfg(target_os = "windows")]
    {
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = fs::remove_file(path);
        fs::rename(&tmp_path, path)
    }
}

fn to_sidecar_path(path_no_ext_or_full: impl AsRef<Path>) -> PathBuf {
    let p = path_no_ext_or_full.as_ref();
    let s = p.to_string_lossy();
    if s.ends_with(".sm.ron") { return p.to_path_buf(); }
    let mut out = p.to_path_buf();
    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
        // If user passed a base without extension, append .sm.ron
        if p.extension().is_none() || p.extension().and_then(|e| e.to_str()) != Some("sm.ron") {
            out.set_file_name(format!("{}.sm.ron", stem));
        }
    } else {
        out.set_file_name("state_machine.sm.ron");
    }
    out
}

pub fn save_sidecar_text(path_no_ext_or_full: impl AsRef<Path>, contents: &str) -> io::Result<()> {
    let path = to_sidecar_path(path_no_ext_or_full);
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    atomic_write(&path, contents)
}


