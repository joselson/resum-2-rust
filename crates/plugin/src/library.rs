//! Where the preset browser looks for presets.

use lumen_engine::tables;
use std::path::{Path, PathBuf};

/// The folder to browse: the `Presets` folder of the loaded preset's library, else
/// the preset's own folder; with no preset, the Serum library from Serum's
/// preferences or the default location. `env` reads environment variables.
pub fn browse_dir(preset: &str, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if !preset.is_empty() {
        let preset = Path::new(preset);
        return tables::infer_root(preset).and_then(|t| t.parent().map(presets_folder)).or_else(|| preset.parent().map(Path::to_path_buf));
    }
    serum_library(env).map(|lib| presets_folder(&lib))
}

/// `<library>/Presets` when it exists, else the library folder itself.
fn presets_folder(library: &Path) -> PathBuf {
    let presets = library.join("Presets");
    if presets.is_dir() {
        presets
    } else {
        library.to_path_buf()
    }
}

/// Serum 2's library: the "Serum Presets Path" preference when set, else
/// `Documents/Xfer/Serum 2 Presets` (also under a OneDrive-redirected Documents).
fn serum_library(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let configured = env("APPDATA").and_then(|appdata| {
        let prefs = std::fs::read(Path::new(&appdata).join("Xfer").join("Serum 2").join("Serum2Prefs.json")).ok()?;
        let prefs: serde_json::Value = serde_json::from_slice(&prefs).ok()?;
        let path = prefs.get("Serum Presets Path")?.as_str()?.trim();
        (!path.is_empty()).then(|| PathBuf::from(path))
    });
    let home = env("USERPROFILE").or_else(|| env("HOME")).map(PathBuf::from);
    let defaults = home.into_iter().flat_map(|h| [h.join("Documents"), h.join("OneDrive").join("Documents")]).map(|d| d.join("Xfer").join("Serum 2 Presets"));
    configured.into_iter().chain(defaults).find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumen-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn falls_back_to_the_default_library_and_honours_serum_prefs() {
        let home = temp("home");
        let presets = home.join("Documents/Xfer/Serum 2 Presets/Presets");
        std::fs::create_dir_all(&presets).unwrap();
        let env = |home: &Path, appdata: Option<&Path>| {
            let (home, appdata) = (home.to_string_lossy().into_owned(), appdata.map(|a| a.to_string_lossy().into_owned()));
            move |k: &str| match k {
                "USERPROFILE" => Some(home.clone()),
                "APPDATA" => appdata.clone(),
                _ => None,
            }
        };
        assert_eq!(browse_dir("", env(&home, None)), Some(presets.clone()));

        let custom = home.join("Elsewhere");
        std::fs::create_dir_all(&custom).unwrap();
        let appdata = home.join("AppData");
        std::fs::create_dir_all(appdata.join("Xfer/Serum 2")).unwrap();
        let prefs = serde_json::json!({ "Serum Presets Path": custom.to_string_lossy() });
        std::fs::write(appdata.join("Xfer/Serum 2/Serum2Prefs.json"), prefs.to_string()).unwrap();
        assert_eq!(browse_dir("", env(&home, Some(&appdata))), Some(custom.clone()), "the preference wins");

        let nowhere = temp("nowhere");
        assert_eq!(browse_dir("", env(&nowhere, None)), None);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_loaded_preset_browses_its_own_library() {
        let lib = temp("lib");
        std::fs::create_dir_all(lib.join("Tables")).unwrap();
        std::fs::create_dir_all(lib.join("Presets/Bass")).unwrap();
        let preset = lib.join("Presets/Bass/X.SerumPreset");
        assert_eq!(browse_dir(&preset.to_string_lossy(), |_| None), Some(lib.join("Presets")));
        std::fs::remove_dir_all(&lib).unwrap();
    }
}
