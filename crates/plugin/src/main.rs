//! Lumen as a standalone app. Audio and MIDI come from the settings saved by the
//! editor's Audio panel; any command-line arguments (`--help` lists them) replace those.

fn main() {
    lumen_plugin::standalone::run();
}
