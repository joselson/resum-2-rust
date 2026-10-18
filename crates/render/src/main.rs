//! Render a preset to WAV, or survey a directory of presets.
//!
//! ```text
//! lumen-render PRESET -o OUT.wav [--tables DIR] [--notes 60,64,67] [--velocity 0..1]
//!              [--hold SECONDS] [--tail SECONDS] [--rate HZ] [--bpm BPM]
//! lumen-render --survey DIR [--tables DIR] [--top N] [--verbose]
//! ```

use lumen_engine::offline::{self, NoteJob};
use lumen_engine::{tables, Coverage, Patch, TableStore};
use lumen_preset::Preset;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

struct Options {
    preset: Option<PathBuf>,
    survey: Option<PathBuf>,
    out: Option<PathBuf>,
    tables: Option<PathBuf>,
    notes: Vec<u8>,
    velocity: f32,
    hold: f32,
    tail: f32,
    rate: u32,
    bpm: f32,
    top: usize,
    verbose: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options { preset: None, survey: None, out: None, tables: None, notes: vec![60], velocity: 0.8, hold: 1.5, tail: 1.5, rate: 48000, bpm: 120.0, top: 25, verbose: false };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        let number = |v: String, name: &str| v.parse::<f32>().map_err(|_| format!("{name}: not a number: {v}"));
        match arg.as_str() {
            "-o" | "--out" => o.out = Some(value("--out")?.into()),
            "--tables" => o.tables = Some(value("--tables")?.into()),
            "--survey" => o.survey = Some(value("--survey")?.into()),
            "--notes" => {
                o.notes = value("--notes")?.split(',').map(|n| n.trim().parse::<u8>().map_err(|_| format!("bad note: {n}"))).collect::<Result<_, _>>()?;
                if o.notes.iter().any(|&n| n > 127) || o.notes.is_empty() {
                    return Err("notes must be 0–127".into());
                }
            }
            "--velocity" => o.velocity = number(value("--velocity")?, "--velocity")?.clamp(0.0, 1.0),
            "--hold" => o.hold = number(value("--hold")?, "--hold")?.max(0.0),
            "--tail" => o.tail = number(value("--tail")?, "--tail")?.max(0.0),
            "--rate" => o.rate = value("--rate")?.parse().map_err(|_| "--rate: not an integer")?,
            "--bpm" => o.bpm = number(value("--bpm")?, "--bpm")?,
            "--top" => o.top = value("--top")?.parse().map_err(|_| "--top: not an integer")?,
            "-v" | "--verbose" => o.verbose = true,
            "-h" | "--help" => return Err(String::new()),
            a if a.starts_with('-') => return Err(format!("unknown option {a}")),
            _ => o.preset = Some(arg.into()),
        }
    }
    if !(8000..=384_000).contains(&o.rate) {
        return Err("--rate must be 8000–384000".into());
    }
    Ok(o)
}

fn load(path: &Path, tables: &mut TableStore) -> Result<(Patch, Coverage), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let preset = Preset::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mut patch, cov) = Patch::from_payload(&preset.payload, tables);
    if let Some(name) = preset.metadata.get("presetName").and_then(|v| v.as_str()) {
        patch.name = name.to_owned();
    }
    Ok((patch, cov))
}

struct Rendered {
    left: Vec<f32>,
    right: Vec<f32>,
}

fn render(patch: Patch, o: &Options) -> Rendered {
    let job = NoteJob { notes: o.notes.clone(), velocity: o.velocity, hold: o.hold, tail: o.tail, sample_rate: o.rate, bpm: o.bpm };
    let (left, right) = offline::render(patch, &job);
    Rendered { left, right }
}

fn write_wav(path: &Path, r: &Rendered, rate: u32) -> Result<(), String> {
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| format!("{}: {e}", path.display()))?;
    for (l, r) in r.left.iter().zip(&r.right) {
        w.write_sample(*l).and_then(|_| w.write_sample(*r)).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

fn stats(r: &Rendered) -> (f32, f32, bool) {
    let finite = r.left.iter().chain(&r.right).all(|x| x.is_finite());
    let peak = r.left.iter().chain(&r.right).fold(0f32, |m, x| m.max(x.abs()));
    let energy: f64 = r.left.iter().chain(&r.right).map(|&x| (x as f64) * (x as f64)).sum();
    let rms = (energy / (2 * r.left.len().max(1)) as f64).sqrt() as f32;
    (peak, rms, finite)
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

fn presets_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                stack.extend(entries.filter_map(|e| e.ok().map(|e| e.path())));
            }
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("SerumPreset")) {
            found.push(p);
        }
    }
    found.sort();
    found
}

fn survey(dir: &Path, o: &Options) -> Result<(), String> {
    let files = presets_under(dir);
    if files.is_empty() {
        return Err(format!("no .SerumPreset files under {}", dir.display()));
    }
    let mut tables = TableStore::new(o.tables.clone().or_else(|| tables::infer_root(&files[0])));
    let mut features: BTreeMap<String, usize> = BTreeMap::new();
    let (mut failed, mut silent, mut clipping, mut nonfinite, mut table_errors, mut fully_covered, mut routes) = (0, 0, 0, 0, 0, 0, 0);
    let started = Instant::now();
    for f in &files {
        if o.verbose {
            eprintln!("{}", f.display());
        }
        let (patch, cov) = match load(f, &mut tables) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("load failed: {e}");
                failed += 1;
                continue;
            }
        };
        let r = render(patch, o);
        let (peak, _, finite) = stats(&r);
        nonfinite += usize::from(!finite);
        silent += usize::from(finite && peak < 1e-4);
        clipping += usize::from(peak > 1.0);
        table_errors += usize::from(!cov.table_errors.is_empty());
        fully_covered += usize::from(cov.unsupported.is_empty() && cov.table_errors.is_empty());
        routes += cov.routes;
        for k in cov.unsupported.keys() {
            *features.entry(k.clone()).or_default() += 1;
        }
    }
    let elapsed = started.elapsed().as_secs_f32();
    let n = files.len();
    println!("presets: {n} ({failed} failed to load) in {elapsed:.1}s");
    println!("renders: {nonfinite} non-finite, {silent} silent, {clipping} peaking above 0 dBFS");
    println!("asset load errors: {table_errors} presets");
    println!("mod routes applied: {routes}");
    println!("fully covered (nothing unsupported): {fully_covered}/{n}");
    let mut ranked: Vec<_> = features.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("\nmost common unsupported features (presets using each):");
    for (k, c) in ranked.iter().take(o.top) {
        println!("  {c:4}  {k}");
    }
    if nonfinite > 0 {
        return Err(format!("{nonfinite} renders produced non-finite samples"));
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let o = parse_args()?;
    if let Some(dir) = &o.survey {
        return survey(dir, &o);
    }
    let preset = o.preset.as_ref().ok_or("missing PRESET")?;
    let out = o.out.as_ref().ok_or("missing -o OUT.wav")?;
    let root = o.tables.clone().or_else(|| tables::infer_root(preset));
    let mut tables = TableStore::new(root);
    let (patch, cov) = load(preset, &mut tables)?;
    let name = patch.name.clone();
    let started = Instant::now();
    let r = render(patch, &o);
    let elapsed = started.elapsed().as_secs_f32();
    let (peak, rms, finite) = stats(&r);
    write_wav(out, &r, o.rate)?;
    let audio = r.left.len() as f32 / o.rate as f32;
    println!("{name}: {audio:.2}s rendered in {elapsed:.3}s ({:.0}x realtime), peak {:.1} dBFS, rms {:.1} dBFS{}", audio / elapsed.max(1e-6), db(peak), db(rms), if finite { "" } else { ", NON-FINITE" });
    println!("mod routes applied: {}", cov.routes);
    for e in &cov.table_errors {
        println!("load error: {e}");
    }
    if !cov.unsupported.is_empty() {
        println!("not rendered yet:");
        for (k, c) in &cov.unsupported {
            println!("  {k}{}", if *c > 1 { format!(" ×{c}") } else { String::new() });
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => {
            println!("usage: lumen-render PRESET -o OUT.wav [--tables DIR] [--notes 60,64,67] [--velocity 0..1] [--hold S] [--tail S] [--rate HZ] [--bpm BPM]\n       lumen-render --survey DIR [--tables DIR] [--top N]");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
