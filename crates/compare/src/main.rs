//! Lumen vs. reference render comparison.
//!
//! ```text
//! lumen-compare jobs   --presets DIR --dir RUN_DIR [--count N] [--notes 48] [--hold S] [--tail S] [--rate HZ]
//! lumen-compare report --dir RUN_DIR [--tables DIR]
//! ```
//!
//! `jobs` writes RUN_DIR/jobs.json. The reference renderer (serum_render.py on
//! Windows) fills RUN_DIR/reference/; `report` renders Lumen into RUN_DIR/lumen/,
//! compares every pair present and writes RUN_DIR/report.md and report.json.

mod analysis;

use analysis::{analyze, compare, Audio, Comparison, Features};
use lumen_engine::offline::{self, NoteJob};
use lumen_engine::{tables, Patch, TableStore};
use lumen_preset::Preset;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Serialize, Deserialize, Clone)]
struct Job {
    id: String,
    /// Path as seen by this tool.
    preset_local: String,
    /// Paths as seen by the Windows reference renderer.
    preset: String,
    out: String,
    notes: Vec<u8>,
    velocity: f32,
    hold: f32,
    tail: f32,
}

#[derive(Serialize, Deserialize)]
struct Jobs {
    rate: u32,
    jobs: Vec<Job>,
}

#[derive(Serialize)]
struct Row {
    id: String,
    preset: String,
    unsupported: Vec<String>,
    reference: Features,
    lumen: Features,
    diff: Comparison,
}

/// `/mnt/c/Users/x` ↔ `C:\Users\x`; other paths are passed through.
fn to_windows(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix("/mnt/") {
        Some(rest) if rest.len() >= 2 && rest.as_bytes()[1] == b'/' => format!("{}:\\{}", rest[..1].to_uppercase(), rest[2..].replace('/', "\\")),
        _ => s.into_owned(),
    }
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

fn slug(p: &Path) -> String {
    p.file_stem().unwrap_or_default().to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

struct Args {
    map: std::collections::HashMap<String, String>,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut map = std::collections::HashMap::new();
        let mut it = args.peekable();
        while let Some(a) = it.next() {
            let key = a.strip_prefix("--").ok_or_else(|| format!("unexpected argument {a}"))?.to_owned();
            let value = it.next().ok_or_else(|| format!("--{key} needs a value"))?;
            map.insert(key, value);
        }
        Ok(Args { map })
    }

    fn get(&self, k: &str) -> Option<&str> {
        self.map.get(k).map(String::as_str)
    }

    fn need(&self, k: &str) -> Result<&str, String> {
        self.get(k).ok_or_else(|| format!("missing --{k}"))
    }

    fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> Result<T, String> {
        self.get(k).map(|v| v.parse().map_err(|_| format!("--{k}: bad value {v}"))).unwrap_or(Ok(default))
    }
}

fn make_jobs(a: &Args) -> Result<(), String> {
    let presets = presets_under(Path::new(a.need("presets")?));
    if presets.is_empty() {
        return Err("no presets found".into());
    }
    let dir = PathBuf::from(a.need("dir")?);
    std::fs::create_dir_all(dir.join("reference")).map_err(|e| e.to_string())?;
    let count: usize = a.num("count", 60)?;
    let notes: Vec<u8> = a.get("notes").unwrap_or("48").split(',').map(|n| n.trim().parse().map_err(|_| format!("bad note {n}"))).collect::<Result<_, _>>()?;
    // Even stride over the sorted list spreads picks across categories deterministically.
    let stride = (presets.len() as f32 / count.min(presets.len()) as f32).max(1.0);
    let jobs: Vec<Job> = (0..count.min(presets.len()))
        .map(|i| {
            let p = &presets[(i as f32 * stride) as usize];
            let id = format!("{i:03}_{}", slug(p));
            Job {
                preset_local: p.to_string_lossy().into_owned(),
                preset: to_windows(p),
                out: to_windows(&dir.join("reference").join(format!("{id}.wav"))),
                id,
                notes: notes.clone(),
                velocity: 0.8,
                hold: 1.5,
                tail: 1.5,
            }
        })
        .collect();
    let jobs = Jobs { rate: a.num("rate", 48000)?, jobs };
    let path = dir.join("jobs.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&jobs).unwrap()).map_err(|e| e.to_string())?;
    println!("wrote {} jobs to {}", jobs.jobs.len(), path.display());
    Ok(())
}

fn read_wav(path: &Path) -> Result<Audio, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = r.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect()
        }
    }
    .map_err(|e| e.to_string())?;
    let ch = spec.channels.max(1) as usize;
    let left = samples.iter().step_by(ch).copied().collect();
    let right = samples.iter().skip(if ch > 1 { 1 } else { 0 }).step_by(ch).copied().collect();
    Ok(Audio { rate: spec.sample_rate, left, right })
}

fn write_wav(path: &Path, audio: &Audio) -> Result<(), String> {
    let spec = hound::WavSpec { channels: 2, sample_rate: audio.rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for (l, r) in audio.left.iter().zip(&audio.right) {
        w.write_sample(*l).and_then(|_| w.write_sample(*r)).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

fn render_lumen(job: &Job, rate: u32, tables_dir: Option<&Path>) -> Result<(Audio, Vec<String>), String> {
    let path = Path::new(&job.preset_local);
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let preset = Preset::read(&bytes).map_err(|e| e.to_string())?;
    let root = tables_dir.map(Path::to_path_buf).or_else(|| tables::infer_root(path));
    let mut store = TableStore::new(root);
    let (patch, cov) = Patch::from_payload(&preset.payload, &mut store);
    let note_job = NoteJob { notes: job.notes.clone(), velocity: job.velocity, hold: job.hold, tail: job.tail, sample_rate: rate, bpm: 120.0 };
    let (left, right) = offline::render(patch, &note_job);
    Ok((Audio { rate, left, right }, cov.unsupported.into_keys().collect()))
}

fn fmt_opt(v: Option<f32>, unit: &str) -> String {
    v.map(|x| format!("{x:+.2}{unit}")).unwrap_or_else(|| "–".into())
}

fn report(a: &Args) -> Result<(), String> {
    let dir = PathBuf::from(a.need("dir")?);
    let jobs: Jobs = serde_json::from_slice(&std::fs::read(dir.join("jobs.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let tables_dir = a.get("tables").map(PathBuf::from);
    std::fs::create_dir_all(dir.join("lumen")).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    let mut missing = Vec::new();
    for job in &jobs.jobs {
        let ref_path = dir.join("reference").join(format!("{}.wav", job.id));
        let Ok(reference) = read_wav(&ref_path) else {
            missing.push(job.id.clone());
            continue;
        };
        let (lumen, unsupported) = render_lumen(job, reference.rate, tables_dir.as_deref())?;
        write_wav(&dir.join("lumen").join(format!("{}.wav", job.id)), &lumen)?;
        let (rf, lf) = (analyze(&reference, job.hold), analyze(&lumen, job.hold));
        let diff = compare(&lf, &rf);
        let preset = Path::new(&job.preset_local).file_stem().unwrap_or_default().to_string_lossy().into_owned();
        rows.push(Row { id: job.id.clone(), preset, unsupported, reference: rf, lumen: lf, diff });
    }
    rows.sort_by(|a, b| b.diff.score.total_cmp(&a.diff.score));
    std::fs::write(dir.join("report.json"), serde_json::to_vec_pretty(&rows).unwrap()).map_err(|e| e.to_string())?;

    let n = rows.len().max(1) as f32;
    let mean = |f: &dyn Fn(&Row) -> f32| rows.iter().map(f).sum::<f32>() / n;
    let mut md = String::new();
    let _ = writeln!(md, "# Lumen vs reference: {} presets\n", rows.len());
    let _ = writeln!(md, "| metric | mean |\n|---|---|");
    let _ = writeln!(md, "| score (0 = match) | {:.2} |", mean(&|r| r.diff.score));
    let _ = writeln!(md, "| loudness Δ (dB, abs) | {:.1} |", mean(&|r| r.diff.loudness_db.abs()));
    let _ = writeln!(md, "| envelope Δ (dB) | {:.1} |", mean(&|r| r.diff.envelope_db));
    let _ = writeln!(md, "| spectral balance Δ (dB/band) | {:.1} |", mean(&|r| r.diff.spectrum_db));
    let _ = writeln!(md, "| centroid Δ (octaves, abs) | {:.2} |", mean(&|r| r.diff.centroid_oct.abs()));
    let pitched: Vec<f32> = rows.iter().filter_map(|r| r.diff.pitch_cents).collect();
    let within = pitched.iter().filter(|c| c.abs() < 10.0).count();
    let _ = writeln!(md, "| pitch within 10 cents | {within}/{} detected |", pitched.len());
    let silent = rows.iter().filter(|r| r.lumen.silent && !r.reference.silent).count();
    let _ = writeln!(md, "| silent in Lumen only | {silent} |");
    if !missing.is_empty() {
        let _ = writeln!(md, "\nNo reference render for {} jobs: {}", missing.len(), missing.join(", "));
    }
    let _ = writeln!(md, "\n| score | preset | loud dB | env dB | spec dB | centroid oct | pitch | attack Δ | release Δ | width dB | unsupported |\n|---|---|---|---|---|---|---|---|---|---|---|");
    for r in &rows {
        let d = &r.diff;
        let _ = writeln!(
            md,
            "| {:.2} | {} | {:+.1} | {:.1} | {:.1} | {:+.2} | {} | {} | {} | {:+.1} | {} |",
            d.score, r.preset, d.loudness_db, d.envelope_db, d.spectrum_db, d.centroid_oct, fmt_opt(d.pitch_cents, "¢"), fmt_opt(d.attack_s, "s"), fmt_opt(d.release_s, "s"), d.width_db, r.unsupported.len()
        );
    }
    std::fs::write(dir.join("report.md"), &md).map_err(|e| e.to_string())?;
    print!("{}", md.split("\n| score | preset").next().unwrap_or(""));
    println!("\nfull table: {}", dir.join("report.md").display());
    Ok(())
}

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1);
    let cmd = argv.next().unwrap_or_default();
    let result = Args::parse(argv).and_then(|a| match cmd.as_str() {
        "jobs" => make_jobs(&a),
        "report" => report(&a),
        _ => Err("usage: lumen-compare jobs --presets DIR --dir RUN_DIR [--count N] | report --dir RUN_DIR".into()),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
