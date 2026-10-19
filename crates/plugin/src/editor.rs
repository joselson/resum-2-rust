//! A basic egui editor: preset browser, output and macros, and a generic page of
//! sliders for every module parameter.

use crate::params::LumenParams;
use crate::{library, piano, Lumen, Shared, Task};
use lumen_engine::params::MODULES;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText};
use nih_plug_egui::{create_egui_editor, widgets::ParamSlider};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct State {
    /// Folder the browser scans.
    browse_dir: String,
    scanned: bool,
    filter: String,
    /// Selected module (index into `MODULES`) and instance.
    module: usize,
    instance: usize,
    search: String,
    /// Lowest octave shown on the keyboard (C4 = 60).
    octave: i32,
    /// Note held with the mouse, and notes held on the computer keyboard by key.
    mouse_note: Option<u8>,
    typed: Vec<(egui::Key, u8)>,
    /// The standalone's audio settings window, with device lists read when it opened.
    #[cfg(feature = "standalone")]
    audio: Option<AudioPanel>,
    /// Modules that have host parameters, in table order, with their instance counts.
    modules: Vec<(usize, usize)>,
}

pub fn create(params: Arc<LumenParams>, shared: Arc<Shared>, executor: AsyncExecutor<Lumen>) -> Option<Box<dyn Editor>> {
    let mut modules: Vec<(usize, usize)> = Vec::new();
    for m in &params.modules {
        if modules.last().map(|&(k, _)| k) != Some(m.target.kind) {
            modules.push((m.target.kind, MODULES[m.target.kind].instances));
        }
    }
    let state = State {
        browse_dir: library::browse_dir(&params.preset_path.read().unwrap(), |k| std::env::var(k).ok()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        scanned: false,
        filter: String::new(),
        module: modules.first().map_or(0, |&(k, _)| k),
        instance: 0,
        search: String::new(),
        octave: 2,
        mouse_note: None,
        typed: Vec::new(),
        #[cfg(feature = "standalone")]
        audio: None,
        modules,
    };
    create_egui_editor(params.editor_state.clone(), state, |_, _| {}, move |ctx, setter, state| {
        if !state.scanned && !state.browse_dir.is_empty() {
            state.scanned = true;
            executor.execute_background(Task::Scan(PathBuf::from(&state.browse_dir)));
        }
        publish_loaded_values(&params, &shared, setter);
        let status = shared.status.lock().unwrap().clone();

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Lumen");
                ui.separator();
                let current = PathBuf::from(&*params.preset_path.read().unwrap());
                let presets = shared.presets.lock().unwrap();
                let at = presets.iter().position(|p| *p == current);
                let step = |delta: isize| {
                    let n = presets.len() as isize;
                    (n > 0).then(|| presets[at.map_or(0, |i| (i as isize + delta).rem_euclid(n) as usize)].clone())
                };
                if ui.button("◀").clicked() {
                    if let Some(p) = step(-1) {
                        executor.execute_background(Task::Load(p));
                    }
                }
                if ui.button("▶").clicked() {
                    if let Some(p) = step(1) {
                        executor.execute_background(Task::Load(p));
                    }
                }
                drop(presets);
                ui.label(RichText::new(&status.name).strong().size(16.0));
                if status.loading {
                    ui.spinner();
                }
                ui.label(RichText::new(&status.message).color(Color32::GRAY));
                #[cfg(feature = "standalone")]
                if crate::standalone::is_standalone() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Audio settings").clicked() {
                            state.audio = if state.audio.is_some() { None } else { Some(AudioPanel::open()) };
                        }
                    });
                }
            });
        });

        #[cfg(feature = "standalone")]
        audio_window(ctx, state);

        egui::TopBottomPanel::bottom("keyboard").show(ctx, |ui| keyboard(ui, &shared, state));

        egui::SidePanel::left("browser").default_width(280.0).show(ctx, |ui| browser(ui, &params, &shared, &executor, state));

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Grid::new("globals").num_columns(6).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Output");
                ui.add(ParamSlider::for_param(&params.gain, setter).with_width(110.0));
                // Two macros per row, beside Output on the first.
                for (i, m) in params.macros.iter().enumerate() {
                    ui.label(format!("Macro {}", i + 1));
                    ui.add(ParamSlider::for_param(m, setter).with_width(110.0));
                    if i % 2 == 1 {
                        ui.end_row();
                        if i + 1 < params.macros.len() {
                            ui.label("");
                            ui.label("");
                        }
                    }
                }
            });
            ui.separator();
            module_page(ui, &params, setter, state);
        });
    })
}

/// Publishes a preset the loader just read to the host, so its parameters,
/// automation lanes and saved state show the preset's values.
fn publish_loaded_values(params: &LumenParams, shared: &Shared, setter: &ParamSetter) {
    let Some(values) = shared.sync.lock().unwrap().take() else { return };
    for (m, n) in params.modules.iter().zip(values) {
        if (m.param.unmodulated_normalized_value() - n).abs() > 1e-6 {
            setter.begin_set_parameter(&m.param);
            setter.set_parameter_normalized(&m.param, n);
            setter.end_set_parameter(&m.param);
        }
    }
}

fn browser(ui: &mut egui::Ui, params: &LumenParams, shared: &Shared, executor: &AsyncExecutor<Lumen>, state: &mut State) {
    ui.label("Preset folder");
    ui.horizontal(|ui| {
        let edit = ui.add(egui::TextEdit::singleline(&mut state.browse_dir).desired_width(200.0));
        if ui.button("Scan").clicked() || edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            executor.execute_background(Task::Scan(PathBuf::from(&state.browse_dir)));
        }
    });
    ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text("Filter presets").desired_width(f32::INFINITY));
    ui.separator();
    let presets = shared.presets.lock().unwrap();
    let filter = state.filter.to_lowercase();
    let root = Path::new(&state.browse_dir);
    let shown: Vec<(&PathBuf, String)> = presets
        .iter()
        .map(|p| (p, p.strip_prefix(root).unwrap_or(p).with_extension("").to_string_lossy().into_owned()))
        .filter(|(_, label)| filter.is_empty() || label.to_lowercase().contains(&filter))
        .collect();
    if presets.is_empty() {
        ui.label(RichText::new("No presets found. Enter the folder holding your .SerumPreset files and press Scan.").color(Color32::GRAY));
    }
    let current = params.preset_path.read().unwrap().clone();
    let row = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show_rows(ui, row, shown.len(), |ui, range| {
        for (path, label) in &shown[range] {
            let selected = path.to_string_lossy() == current;
            if ui.selectable_label(selected, label.as_str()).clicked() {
                executor.execute_background(Task::Load((*path).clone()));
            }
        }
    });
}

fn module_page(ui: &mut egui::Ui, params: &LumenParams, setter: &ParamSetter, state: &mut State) {
    ui.horizontal_wrapped(|ui| {
        for &(kind, _) in &state.modules {
            if ui.selectable_label(state.module == kind && state.search.is_empty(), MODULES[kind].name).clicked() {
                state.module = kind;
                state.instance = 0;
                state.search.clear();
            }
        }
    });
    ui.horizontal(|ui| {
        let instances = state.modules.iter().find(|&&(k, _)| k == state.module).map_or(1, |&(_, n)| n);
        if instances > 1 && state.search.is_empty() {
            for i in 0..instances {
                ui.selectable_value(&mut state.instance, i, format!("{} {}", MODULES[state.module].name, i + 1));
            }
            ui.separator();
        }
        ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("Find parameter").desired_width(180.0));
    });
    ui.separator();
    let search = state.search.to_lowercase();
    let shown: Vec<_> = if search.is_empty() {
        params.modules.iter().filter(|m| m.target.kind == state.module && m.target.instance == state.instance).collect()
    } else {
        params.modules.iter().filter(|m| m.param.name().to_lowercase().contains(&search)).take(200).collect()
    };
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        egui::Grid::new("module-params").num_columns(4).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
            for (i, m) in shown.iter().enumerate() {
                let label = if search.is_empty() { m.name.clone() } else { m.param.name().to_owned() };
                ui.label(label).on_hover_text(&m.id);
                ui.add(ParamSlider::for_param(&m.param, setter).with_width(150.0));
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    });
}

const OCTAVES: usize = 4;

fn send(shared: &Shared, note: lumen_engine::notes::NoteEvent) {
    shared.keys.lock().unwrap().push(note);
}

fn note_on(shared: &Shared, note: u8, velocity: f32) {
    send(shared, lumen_engine::notes::NoteEvent::On { note, velocity });
}

fn note_off(shared: &Shared, note: u8) {
    send(shared, lumen_engine::notes::NoteEvent::Off { note, velocity: 0.5 });
}

/// Maps an egui key to the character `piano::typed_offset` expects.
fn key_char(key: egui::Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_alphabetic()).then_some(c)
}

/// A playable keyboard: mouse (lower on a key is louder) or computer keys
/// `A W S E D F T G Y H U J K O L P`, with `Z`/`X` for octave down/up.
fn keyboard(ui: &mut egui::Ui, shared: &Shared, state: &mut State) {
    // In a DAW the host decides who gets keys; the standalone hands them to the editor.
    #[cfg(feature = "standalone")]
    if crate::standalone::is_standalone() {
        crate::standalone::claim_keyboard();
    }
    let base = piano::base_note(state.octave, OCTAVES);
    ui.horizontal(|ui| {
        if ui.button("Oct −").clicked() {
            state.octave -= 1;
        }
        if ui.button("Oct +").clicked() {
            state.octave += 1;
        }
        ui.label(format!("C{} – C{}", base as i32 / 12 - 1, base as i32 / 12 - 1 + OCTAVES as i32));
        ui.label(RichText::new("Mouse or keys A W S E D F T G Y H U J K O L P · Z/X octave").color(Color32::GRAY));
    });

    // Computer keyboard, unless a text field has focus.
    let typing = ui.ctx().memory(|m| m.focused().is_none());
    let events = ui.input(|i| i.events.clone());
    for event in events {
        match event {
            egui::Event::Key { key, pressed, repeat: false, .. } if typing => {
                let Some(c) = key_char(key) else { continue };
                if pressed {
                    match c.to_ascii_lowercase() {
                        'z' => state.octave -= 1,
                        'x' => state.octave += 1,
                        _ => {
                            if let Some(offset) = piano::typed_offset(c) {
                                if !state.typed.iter().any(|&(k, _)| k == key) {
                                    let note = (base + offset).min(127);
                                    note_on(shared, note, 0.8);
                                    state.typed.push((key, note));
                                }
                            }
                        }
                    }
                } else if let Some(i) = state.typed.iter().position(|&(k, _)| k == key) {
                    note_off(shared, state.typed.swap_remove(i).1);
                }
            }
            // Key-ups are lost while the window is unfocused: release everything.
            egui::Event::WindowFocused(false) => {
                for (_, note) in state.typed.drain(..) {
                    note_off(shared, note);
                }
            }
            _ => {}
        }
    }
    state.octave = state.octave.clamp(-1, 9 - OCTAVES as i32);

    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 90.0), egui::Sense::click_and_drag());
    let key_width = rect.width() / piano::white_count(OCTAVES) as f32;
    let hit = |p: egui::Pos2| {
        let (x, y) = ((p.x - rect.left()) / key_width, (p.y - rect.top()) / rect.height());
        piano::key_at(OCTAVES, x, y).map(|offset| (base + offset, (0.3 + 0.7 * y).clamp(0.0, 1.0)))
    };
    let target = if ui.input(|i| i.pointer.primary_down()) { response.interact_pointer_pos().and_then(hit) } else { None };
    if target.map(|(n, _)| n) != state.mouse_note {
        if let Some(n) = state.mouse_note.take() {
            note_off(shared, n);
        }
        if let Some((n, velocity)) = target {
            note_on(shared, n, velocity);
            state.mouse_note = Some(n);
        }
    }

    let held = |n: u8| state.mouse_note == Some(n) || state.typed.iter().any(|&(_, t)| t == n);
    let accent = Color32::from_rgb(70, 150, 220);
    let painter = ui.painter_at(rect);
    for i in 0..piano::white_count(OCTAVES) {
        let note = base + piano::white_offset(i);
        let key = egui::Rect::from_min_size(rect.left_top() + egui::vec2(i as f32 * key_width, 0.0), egui::vec2(key_width, rect.height())).shrink(1.0);
        painter.rect_filled(key, 3.0, if held(note) { accent } else { Color32::from_gray(225) });
        if i % 7 == 0 {
            painter.text(key.center_bottom() - egui::vec2(0.0, 4.0), egui::Align2::CENTER_BOTTOM, format!("C{}", note as i32 / 12 - 1), egui::FontId::proportional(10.0), Color32::from_gray(90));
        }
    }
    for (offset, centre) in piano::black_keys(OCTAVES) {
        let w = key_width * piano::BLACK_WIDTH;
        let key = egui::Rect::from_min_size(rect.left_top() + egui::vec2(centre * key_width - w / 2.0, 0.0), egui::vec2(w, rect.height() * piano::BLACK_HEIGHT));
        painter.rect_filled(key, 2.0, if held(base + offset) { accent } else { Color32::from_gray(30) });
    }
}

#[cfg(feature = "standalone")]
struct AudioPanel {
    settings: crate::standalone::Settings,
    devices: Vec<String>,
    midi: Vec<String>,
    error: Option<String>,
}

#[cfg(feature = "standalone")]
impl AudioPanel {
    fn open() -> AudioPanel {
        use crate::standalone::*;
        AudioPanel { settings: Settings::load(), devices: output_devices(), midi: midi_inputs(), error: None }
    }
}

/// Output device, buffer size and MIDI input; applying saves them and restarts
/// the app, since the standalone wrapper fixes its devices at startup.
#[cfg(feature = "standalone")]
fn audio_window(ctx: &egui::Context, state: &mut State) {
    use crate::standalone;
    let Some(panel) = &mut state.audio else { return };
    let mut open = true;
    egui::Window::new("Audio settings").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.label(RichText::new(format!("Now: {}", standalone::active())).color(Color32::GRAY));
        ui.separator();
        egui::Grid::new("audio").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Output");
            let shown = panel.settings.output_device.clone().unwrap_or_else(|| "System default".into());
            egui::ComboBox::from_id_salt("output").selected_text(shown).width(280.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut panel.settings.output_device, None, "System default");
                for d in &panel.devices {
                    ui.selectable_value(&mut panel.settings.output_device, Some(d.clone()), d);
                }
            });
            ui.end_row();
            ui.label("MIDI input");
            let shown = panel.settings.midi_input.clone().unwrap_or_else(|| "None".into());
            egui::ComboBox::from_id_salt("midi").selected_text(shown).width(280.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut panel.settings.midi_input, None, "None");
                for m in &panel.midi {
                    ui.selectable_value(&mut panel.settings.midi_input, Some(m.clone()), m);
                }
            });
            ui.end_row();
        });
        ui.label(RichText::new("Sample rate and latency follow the device's Windows sound settings.").color(Color32::GRAY));
        ui.separator();
        if ui.button("Apply & restart").clicked() {
            match standalone::relaunch(&panel.settings) {
                Ok(()) => std::process::exit(0),
                Err(e) => panel.error = Some(e),
            }
        }
        if let Some(e) = &panel.error {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
    });
    if !open {
        state.audio = None;
    }
}
