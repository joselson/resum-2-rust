//! WAV reading for wavetables: PCM 8/16/24/32 and IEEE float 32/64, plus the
//! `clm ` chunk that wavetable editors use to declare the frame length.

use std::fmt;

#[derive(Debug)]
pub enum WavError {
    NotRiffWave,
    MissingChunk(&'static str),
    Unsupported { format: u16, bits: u16 },
    Malformed(&'static str),
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WavError::NotRiffWave => write!(f, "not a RIFF/WAVE file"),
            WavError::MissingChunk(c) => write!(f, "missing {c} chunk"),
            WavError::Unsupported { format, bits } => write!(f, "unsupported WAV encoding (format {format}, {bits} bits)"),
            WavError::Malformed(what) => write!(f, "malformed WAV: {what}"),
        }
    }
}

impl std::error::Error for WavError {}

pub struct WavData {
    pub sample_rate: u32,
    /// One vector per channel.
    pub channels: Vec<Vec<f32>>,
    /// Frame length declared by a `clm ` chunk (`<!>2048 ...`), if present.
    pub clm_frame_len: Option<usize>,
}

pub fn read(bytes: &[u8]) -> Result<WavData, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotRiffWave);
    }
    let mut fmt = None;
    let mut data = None;
    let mut clm = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        // Tolerate a final chunk whose declared length overruns the file (common in the wild).
        let body = &bytes[body_start..body_start.saturating_add(len).min(bytes.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            b"clm " => clm = Some(body),
            _ => {}
        }
        pos = body_start.saturating_add(len).saturating_add(len & 1);
    }
    let fmt = fmt.ok_or(WavError::MissingChunk("fmt "))?;
    let data = data.ok_or(WavError::MissingChunk("data"))?;
    if fmt.len() < 16 {
        return Err(WavError::Malformed("short fmt chunk"));
    }
    let mut format = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]).max(1) as usize;
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    // WAVE_FORMAT_EXTENSIBLE carries the real format in the sub-format GUID.
    if format == 0xfffe && fmt.len() >= 26 {
        format = u16::from_le_bytes([fmt[24], fmt[25]]);
    }
    let width = (bits as usize).div_ceil(8);
    if width == 0 {
        return Err(WavError::Unsupported { format, bits });
    }
    let stride = width * channels;
    let decode: fn(&[u8]) -> f32 = match (format, bits) {
        (1, 8) => |b| (b[0] as f32 - 128.0) / 128.0,
        (1, 16) => |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
        (1, 24) => |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
        (1, 32) => |b| i32::from_le_bytes(b[..4].try_into().unwrap()) as f32 / 2_147_483_648.0,
        (3, 32) => |b| f32::from_le_bytes(b[..4].try_into().unwrap()),
        (3, 64) => |b| f64::from_le_bytes(b[..8].try_into().unwrap()) as f32,
        _ => return Err(WavError::Unsupported { format, bits }),
    };
    let mut out = vec![Vec::with_capacity(data.len() / stride); channels];
    for frame in data.chunks_exact(stride) {
        for (ch, buf) in out.iter_mut().enumerate() {
            buf.push(decode(&frame[ch * width..(ch + 1) * width]));
        }
    }
    let clm_frame_len = clm.and_then(parse_clm);
    Ok(WavData { sample_rate, channels: out, clm_frame_len })
}

fn parse_clm(body: &[u8]) -> Option<usize> {
    let text = body.strip_prefix(b"<!>")?;
    let digits: Vec<u8> = text.iter().copied().take_while(u8::is_ascii_digit).collect();
    std::str::from_utf8(&digits).ok()?.parse().ok().filter(|&n: &usize| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(format: u16, bits: u16, extra: &[u8], data: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend(format.to_le_bytes());
        fmt.extend(1u16.to_le_bytes());
        fmt.extend(44100u32.to_le_bytes());
        fmt.extend((44100 * bits as u32 / 8).to_le_bytes());
        fmt.extend((bits / 8).to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        let mut body = b"WAVE".to_vec();
        for (id, chunk) in [(b"fmt ", fmt.as_slice()), (b"clm ", extra), (b"data", data)] {
            if chunk.is_empty() && id == b"clm " {
                continue;
            }
            body.extend(id);
            body.extend((chunk.len() as u32).to_le_bytes());
            body.extend(chunk);
        }
        let mut out = b"RIFF".to_vec();
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn reads_float_with_clm_frame_length() {
        let data: Vec<u8> = [0.5f32, -0.25].iter().flat_map(|s| s.to_le_bytes()).collect();
        let w = read(&wav(3, 32, b"<!>2048 01000000 wavetable", &data)).unwrap();
        assert_eq!(w.channels, vec![vec![0.5, -0.25]]);
        assert_eq!(w.clm_frame_len, Some(2048));
    }

    #[test]
    fn reads_pcm16_without_clm() {
        let data: Vec<u8> = [16384i16, -32768].iter().flat_map(|s| s.to_le_bytes()).collect();
        let w = read(&wav(1, 16, b"", &data)).unwrap();
        assert_eq!(w.channels, vec![vec![0.5, -1.0]]);
        assert_eq!(w.clm_frame_len, None);
    }

    #[test]
    fn rejects_non_wav() {
        assert!(matches!(read(b"RIFF\0\0\0\0AVI "), Err(WavError::NotRiffWave)));
    }
}
