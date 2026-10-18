//! Wavetable lookup relative to a user-supplied tables directory.

use lumen_dsp::sampler::SampleData;
use lumen_dsp::wavetable::{Wavetable, FRAME_LEN};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Weak};

/// Loads library content (wavetables, samples, impulses) relative to a preset
/// library: `root` is its `Tables` directory; samples and impulses resolve against
/// sibling directories.
pub struct TableStore {
    root: Option<PathBuf>,
    cache: HashMap<String, Arc<Wavetable>>,
    /// Weak, so large sample sets are freed once no patch uses them.
    audio: HashMap<PathBuf, Weak<SampleData>>,
    fallback: Arc<Wavetable>,
}

/// Upper bound on decoded audio length (frames per channel), ~60 s at 48 kHz.
const MAX_AUDIO_FRAMES: usize = 2_880_000;

/// Decodes FLAC or WAV into channel vectors (at most two).
fn decode_audio(path: &Path) -> Result<SampleData, String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let is_flac = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("flac"));
    if !is_flac {
        let bytes = std::fs::read(path).map_err(|e| err(&e))?;
        let wav = lumen_dsp::wav::read(&bytes).map_err(|e| err(&e))?;
        let mut channels = wav.channels;
        channels.truncate(2);
        channels.iter_mut().for_each(|c| c.truncate(MAX_AUDIO_FRAMES));
        return Ok(SampleData { rate: wav.sample_rate as f32, channels });
    }
    let mut reader = claxon::FlacReader::open(path).map_err(|e| err(&e))?;
    let info = reader.streaminfo();
    let source_channels = info.channels.max(1) as usize;
    let kept = source_channels.min(2);
    let scale = 1.0 / (1i64 << (info.bits_per_sample.max(1) - 1)) as f32;
    let mut channels = vec![Vec::new(); kept];
    for (i, sample) in reader.samples().enumerate() {
        let s = sample.map_err(|e| err(&e))?;
        let ch = i % source_channels;
        if ch < kept {
            if channels[ch].len() >= MAX_AUDIO_FRAMES {
                break;
            }
            channels[ch].push(s as f32 * scale);
        }
    }
    Ok(SampleData { rate: info.sample_rate as f32, channels })
}

impl TableStore {
    /// `root` is the directory preset paths are relative to (e.g. `.../Serum 2 Presets/Tables`).
    pub fn new(root: Option<PathBuf>) -> Self {
        TableStore { root, cache: HashMap::new(), audio: HashMap::new(), fallback: Arc::new(Wavetable::basic_shapes()) }
    }

    pub fn fallback(&self) -> Arc<Wavetable> {
        self.fallback.clone()
    }

    /// Loads a table by the preset's relative path, caching successes.
    pub fn load(&mut self, relative: &str) -> Result<Arc<Wavetable>, String> {
        if let Some(t) = self.cache.get(relative) {
            return Ok(t.clone());
        }
        let root = self.root.as_ref().ok_or("no tables directory configured")?;
        let path = resolve(root, relative)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let wav = lumen_dsp::wav::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let frame_len = wav.clm_frame_len.unwrap_or(FRAME_LEN);
        let table = Wavetable::from_samples(&wav.channels[0], frame_len).ok_or_else(|| format!("{}: shorter than one frame", path.display()))?;
        let table = Arc::new(table);
        self.cache.insert(relative.to_owned(), table.clone());
        Ok(table)
    }

    /// The preset library directory (parent of `Tables`).
    fn library(&self) -> Result<&Path, String> {
        self.root.as_deref().and_then(Path::parent).ok_or_else(|| "no preset library configured".to_owned())
    }

    /// Loads audio at `relative` inside the library's `dir` (`Samples`, `Impulses`, …).
    /// The path may step between library folders with `..` but never leave the library.
    pub fn audio(&mut self, dir: &str, relative: &str) -> Result<Arc<SampleData>, String> {
        let library = self.library()?.to_path_buf();
        let path = resolve_within(&library, &Path::new(dir).join(relative.replace('\\', "/")))?;
        if let Some(data) = self.audio.get(&path).and_then(Weak::upgrade) {
            return Ok(data);
        }
        let data = Arc::new(decode_audio(&path)?);
        self.audio.insert(path, Arc::downgrade(&data));
        Ok(data)
    }

    /// Impulse responses live in the library's `Impulses` directory.
    pub fn impulse(&mut self, relative: &str) -> Result<Arc<SampleData>, String> {
        self.audio("Impulses", relative)
    }
}

/// Joins `relative` under `root`, resolving `..` lexically and rejecting any path
/// that would leave `root` (or is absolute).
fn resolve_within(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut out = root.to_path_buf();
    let mut depth = 0usize;
    for c in relative.components() {
        match c {
            Component::Normal(p) => {
                out.push(p);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => {
                out.pop();
                depth -= 1;
            }
            _ => return Err(format!("refusing path outside the preset library: {}", relative.display())),
        }
    }
    Ok(out)
}

/// Finds the `Tables` directory of a preset library by walking up from a preset file
/// (`.../Serum 2 Presets/Presets/Factory/Pad/x.SerumPreset` → `.../Serum 2 Presets/Tables`).
pub fn infer_root(preset: &Path) -> Option<PathBuf> {
    preset.ancestors().skip(1).map(|dir| dir.join("Tables")).find(|t| t.is_dir())
}

/// A wavetable path from a preset, under the `Tables` directory. Some presets root
/// paths at that directory ("/Analog/Basic Shapes.wav").
fn resolve(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let normalized = relative.replace('\\', "/");
    resolve_within(root, Path::new(normalized.trim_start_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escaping_paths() {
        let root = Path::new("/tables");
        assert!(resolve(root, "../secret.wav").is_err());
        assert!(resolve(root, "/../etc/passwd").is_err());
        assert!(resolve(root, "a\\..\\..\\b.wav").is_err());
        assert_eq!(resolve(root, "S2 Tables\\Analog\\x.wav").unwrap(), Path::new("/tables/S2 Tables/Analog/x.wav"));
        assert_eq!(resolve(root, "/Analog/Basic Shapes.wav").unwrap(), Path::new("/tables/Analog/Basic Shapes.wav"));
    }

    #[test]
    fn library_paths_may_move_between_folders_but_not_leave() {
        let root = Path::new("/lib");
        assert_eq!(resolve_within(root, Path::new("Samples/../Multisamples/Keys/a.flac")).unwrap(), Path::new("/lib/Multisamples/Keys/a.flac"));
        assert!(resolve_within(root, Path::new("Samples/../../etc/passwd")).is_err());
        assert!(resolve_within(root, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn missing_root_reports_error() {
        let mut store = TableStore::new(None);
        assert!(store.load("x.wav").is_err());
    }
}
