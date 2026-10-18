//! The SFZ subset Serum 2 multisamples use (embedded in presets as text).

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct SfzRegion {
    /// Sample path relative to the .sfz file, `/`-separated.
    pub sample: String,
    pub lokey: u8,
    pub hikey: u8,
    pub lovel: u8,
    pub hivel: u8,
    pub pitch_keycenter: u8,
    /// Cents.
    pub tune: f32,
    /// dB.
    pub volume: f32,
    /// −100…100.
    pub pan: f32,
    /// 0–100 %.
    pub amp_veltrack: f32,
    pub offset: u64,
    /// Last frame played (inclusive); None = sample end.
    pub end: Option<u64>,
    pub loop_mode: Option<String>,
    pub loop_start: Option<u64>,
    pub loop_end: Option<u64>,
    /// Seconds.
    pub loop_crossfade: f32,
    pub seq_length: u32,
    pub seq_position: u32,
    pub ampeg: [f32; 5],
}

/// Note number from a number or a name (`c4` = 60, `c#4`, `db4`).
fn note(v: &str) -> Option<u8> {
    if let Ok(n) = v.parse::<i32>() {
        return Some(n.clamp(0, 127) as u8);
    }
    let v = v.to_ascii_lowercase();
    let mut chars = v.chars();
    let base = match chars.next()? {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (acc, oct) = if let Some(r) = rest.strip_prefix('#') { (1, r) } else if let Some(r) = rest.strip_prefix('b') { (-1, r) } else { (0, rest.as_str()) };
    let octave: i32 = oct.parse().ok()?;
    Some((12 * (octave + 1) + base + acc).clamp(0, 127) as u8)
}

/// Parses `<group>`/`<region>` headers and opcodes; region opcodes override group ones.
pub fn parse(text: &str) -> Vec<SfzRegion> {
    let mut regions = Vec::new();
    let mut group: HashMap<String, String> = HashMap::new();
    let mut current: Option<HashMap<String, String>> = None;
    let mut header = "";
    // Tokens: headers and key=value pairs; values may contain spaces (sample paths),
    // so a value runs until the next `key=` or header.
    let flush = |current: &mut Option<HashMap<String, String>>, group: &HashMap<String, String>, regions: &mut Vec<SfzRegion>| {
        if let Some(r) = current.take() {
            let mut merged = group.clone();
            merged.extend(r);
            if let Some(region) = build(&merged) {
                regions.push(region);
            }
        }
    };
    for line in text.lines() {
        let line = strip_comment(line).trim();
        let mut rest = line;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix('<') {
                let Some(close) = after.find('>') else { break };
                let name = &after[..close];
                rest = after[close + 1..].trim_start();
                flush(&mut current, &group, &mut regions);
                match name {
                    "group" | "global" | "master" => {
                        if name == "group" {
                            group.clear();
                        }
                        header = "group";
                    }
                    "region" => {
                        header = "region";
                        current = Some(HashMap::new());
                    }
                    _ => header = "",
                }
                continue;
            }
            let Some(eq) = rest.find('=') else { break };
            let key = rest[..eq].trim().to_owned();
            let value_part = &rest[eq + 1..];
            let end = value_end(value_part);
            let value = value_part[..end].trim().to_owned();
            rest = value_part[end..].trim_start();
            match header {
                "region" => {
                    current.get_or_insert_with(HashMap::new).insert(key, value);
                }
                "group" => {
                    group.insert(key, value);
                }
                _ => {}
            }
        }
    }
    flush(&mut current, &group, &mut regions);
    regions
}

/// Removes a `//` comment that starts the line or follows whitespace; `//` inside a
/// value (factory paths like `Samples//XFoo.flac`) is kept.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'/' && bytes[i + 1] == b'/' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &line[..i];
        }
    }
    line
}

/// A value (which may contain spaces, e.g. sample paths) runs until the next
/// ` opcode=` token or `<header>`.
fn value_end(s: &str) -> usize {
    let header = s.find('<').unwrap_or(s.len());
    let bytes = s.as_bytes();
    for i in 0..header {
        if bytes[i] != b' ' {
            continue;
        }
        let word = s[i + 1..].bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
        if word > 0 && bytes.get(i + 1 + word) == Some(&b'=') {
            return i;
        }
    }
    header
}

fn build(op: &HashMap<String, String>) -> Option<SfzRegion> {
    let sample = op.get("sample")?.replace('\\', "/");
    let num = |k: &str| op.get(k).and_then(|v| v.parse::<f32>().ok());
    let int = |k: &str| op.get(k).and_then(|v| v.parse::<f64>().ok()).map(|v| v.max(0.0) as u64);
    let key = op.get("key").and_then(|v| note(v));
    let lokey = op.get("lokey").and_then(|v| note(v)).or(key).unwrap_or(0);
    let hikey = op.get("hikey").and_then(|v| note(v)).or(key).unwrap_or(127);
    let pitch_keycenter = op.get("pitch_keycenter").and_then(|v| note(v)).or(key).unwrap_or(60);
    Some(SfzRegion {
        sample,
        lokey,
        hikey,
        lovel: num("lovel").unwrap_or(1.0).clamp(0.0, 127.0) as u8,
        hivel: num("hivel").unwrap_or(127.0).clamp(0.0, 127.0) as u8,
        pitch_keycenter,
        tune: num("tune").unwrap_or(0.0),
        volume: num("volume").unwrap_or(0.0),
        pan: num("pan").unwrap_or(0.0),
        amp_veltrack: num("amp_veltrack").unwrap_or(100.0),
        offset: int("offset").unwrap_or(0),
        end: int("end"),
        loop_mode: op.get("loop_mode").or(op.get("loopmode")).cloned(),
        loop_start: int("loop_start").or(int("loopstart")),
        loop_end: int("loop_end").or(int("loopend")),
        loop_crossfade: num("loop_crossfade").unwrap_or(0.0),
        seq_length: num("seq_length").unwrap_or(1.0).max(1.0) as u32,
        seq_position: num("seq_position").unwrap_or(1.0).max(1.0) as u32,
        ampeg: [
            num("ampeg_attack").unwrap_or(0.0),
            num("ampeg_hold").unwrap_or(0.0),
            num("ampeg_decay").unwrap_or(0.0),
            num("ampeg_sustain").unwrap_or(100.0) / 100.0,
            num("ampeg_release").unwrap_or(0.001),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_factory_style_sfz() {
        let text = "// SFZ Generated by libSFZ\n<group> amp_veltrack=38.500000 ampeg_release=0.100000\n<region> hikey=38 pitch_keycenter=36 end=173747 loop_start=30305 loop_end=169438 sample=Arp Solina - Horn Samples/XFArSol Horn C1.flac\n<region> lokey=39 hikey=41 pitch_keycenter=40 sample=Oud Samples\\XFOud Kt 01 E1.flac\n";
        let r = parse(text);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].sample, "Arp Solina - Horn Samples/XFArSol Horn C1.flac");
        assert_eq!((r[0].lokey, r[0].hikey, r[0].pitch_keycenter), (0, 38, 36));
        assert_eq!((r[0].loop_start, r[0].loop_end, r[0].end), (Some(30305), Some(169438), Some(173747)));
        assert_eq!(r[0].amp_veltrack, 38.5, "group opcodes are inherited");
        assert!((r[0].ampeg[4] - 0.1).abs() < 1e-6);
        assert_eq!(r[1].sample, "Oud Samples/XFOud Kt 01 E1.flac");
        assert_eq!(r[1].lokey, 39);
    }

    #[test]
    fn sample_paths_keep_spaces_and_dashes() {
        let r = parse("<group> loop_crossfade=0.010000 loop_mode=loop_continuous\n<region> hikey=25 pitch_keycenter=24 lovel=1 hivel=34 loop_start=78992 loop_end=267105 ampeg_attack=0.010000 sample=Full Strings Long Samples/XFfull strings long-01 01 C0.flac\n");
        assert_eq!(r[0].sample, "Full Strings Long Samples/XFfull strings long-01 01 C0.flac");
    }

    #[test]
    fn double_slashes_in_paths_are_not_comments() {
        let r = parse("// header comment\n<region> key=40 sample=Choir Samples//XFChoir O Low E1.flac // trailing\n");
        assert_eq!(r[0].sample, "Choir Samples//XFChoir O Low E1.flac");
    }

    #[test]
    fn note_names() {
        assert_eq!(note("c4"), Some(60));
        assert_eq!(note("C#4"), Some(61));
        assert_eq!(note("eb3"), Some(51));
        assert_eq!(note("a-1"), Some(9));
    }
}
