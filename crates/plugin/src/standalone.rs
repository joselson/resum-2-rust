//! Audio settings for the standalone app. nih-plug's standalone wrapper takes its
//! device setup from command-line arguments and requests 48 kHz unless told
//! otherwise; WASAPI shared-mode devices accept only their own mix rate, so the
//! launcher asks the device for its rate and builds the arguments from saved
//! settings. The editor's settings panel saves new settings and relaunches.

use cpal::traits::{DeviceTrait, HostTrait};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static STANDALONE: AtomicBool = AtomicBool::new(false);
/// What the launcher chose, for the editor to show.
static ACTIVE: OnceLock<String> = OnceLock::new();

pub fn is_standalone() -> bool {
    STANDALONE.load(Ordering::Relaxed)
}

pub fn active() -> &'static str {
    ACTIVE.get().map_or("", String::as_str)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// Output device name; None = the system default.
    pub output_device: Option<String>,
    /// MIDI input port name; None = no MIDI input.
    pub midi_input: Option<String>,
}

/// The wrapper's period is the largest block it accepts per callback. WASAPI shared
/// mode ignores the requested size and delivers the engine's own period (970 frames
/// at 44.1 kHz was observed), so this is a ceiling, not the latency.
const MAX_BLOCK: u32 = 4096;

fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("Lumen").join("standalone.json"))
}

impl Settings {
    fn from_json(v: &serde_json::Value) -> Settings {
        let text = |k: &str| v.get(k).and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
        Settings { output_device: text("output_device"), midi_input: text("midi_input") }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "output_device": self.output_device, "midi_input": self.midi_input })
    }

    pub fn load() -> Settings {
        settings_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .map(|v| Settings::from_json(&v))
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = settings_path().ok_or("no settings folder (APPDATA/HOME unset)")?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, serde_json::to_vec_pretty(&self.to_json()).unwrap()).map_err(|e| e.to_string())
    }
}

pub fn output_devices() -> Vec<String> {
    cpal::default_host().output_devices().map(|ds| ds.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

pub fn midi_inputs() -> Vec<String> {
    midir::MidiInput::new("Lumen").map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()).unwrap_or_default()
}

/// The output device to use and its shared-mode sample rate. Falls back to the
/// default device when the saved one is gone.
fn resolve_output(settings: &Settings) -> Option<(String, u32)> {
    let host = cpal::default_host();
    let named = settings.output_device.as_ref().and_then(|name| host.output_devices().ok()?.find(|d| d.name().ok().as_ref() == Some(name)));
    let device = named.or_else(|| host.default_output_device())?;
    let rate = device.default_output_config().ok()?.sample_rate().0;
    Some((device.name().ok()?, rate))
}

/// Arguments for nih-plug's standalone wrapper. Explicit command-line arguments
/// take precedence over saved settings.
fn arguments(settings: &Settings) -> Vec<String> {
    let given: Vec<String> = std::env::args().collect();
    if given.len() > 1 {
        let _ = ACTIVE.set("command-line settings".into());
        return given;
    }
    let mut args = vec![given.first().cloned().unwrap_or_else(|| "lumen-standalone".into())];
    let mut active = String::from("no audio device found");
    if let Some((device, rate)) = resolve_output(settings) {
        if cfg!(windows) {
            args.extend(["--backend".into(), "wasapi".into()]);
        }
        args.extend(["--output-device".into(), device.clone(), "--sample-rate".into(), rate.to_string(), "--period-size".into(), MAX_BLOCK.to_string()]);
        active = format!("{device} · {rate} Hz");
    }
    // Only pass a MIDI port that exists: an unknown one makes the wrapper give up on audio too.
    if let Some(port) = settings.midi_input.as_ref().filter(|p| midi_inputs().contains(p)) {
        args.extend(["--midi-input".into(), port.clone()]);
        active.push_str(&format!(" · MIDI {port}"));
    }
    let _ = ACTIVE.set(active);
    args
}

pub fn run() {
    STANDALONE.store(true, Ordering::Relaxed);
    let args = arguments(&Settings::load());
    nih_plug::wrapper::standalone::nih_export_standalone_with_args::<crate::Lumen, _>(args);
}

/// Saves the settings and starts a new instance with them; the caller exits.
pub fn relaunch(settings: &Settings) -> Result<(), String> {
    settings.save()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_reject_bad_values() {
        let s = Settings { output_device: Some("Speakers".into()), midi_input: Some("Keys".into()) };
        assert_eq!(Settings::from_json(&s.to_json()), s);
        assert_eq!(Settings::from_json(&serde_json::json!({ "output_device": "", "midi_input": 3 })), Settings::default());
    }
}
