//! Layout of the editor's on-screen keyboard and its computer-keyboard mapping,
//! kept free of GUI types so it can be tested anywhere.

/// Semitone offsets of the white keys within an octave.
const WHITE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
/// Black keys sit on the boundary after these white keys (C, D, F, G, A).
const BLACK_AFTER: [usize; 5] = [0, 1, 3, 4, 5];
/// Black keys cover this fraction of the keyboard height and of a white key's width.
pub const BLACK_HEIGHT: f32 = 0.6;
pub const BLACK_WIDTH: f32 = 0.6;

/// White keys for `octaves` octaves plus the closing C.
pub fn white_count(octaves: usize) -> usize {
    octaves * 7 + 1
}

/// Semitone offset (from the lowest C) of white key `i`.
pub fn white_offset(i: usize) -> u8 {
    (12 * (i / 7)) as u8 + WHITE[i % 7]
}

/// Black keys as (semitone offset, centre in white-key widths from the left edge).
pub fn black_keys(octaves: usize) -> impl Iterator<Item = (u8, f32)> {
    (0..octaves).flat_map(|o| BLACK_AFTER.iter().map(move |&w| (white_offset(o * 7 + w) + 1, (o * 7 + w + 1) as f32)))
}

/// The key under a point, as a semitone offset from the lowest C. `x` is in
/// white-key widths from the left edge, `y` in [0, 1] from the top.
pub fn key_at(octaves: usize, x: f32, y: f32) -> Option<u8> {
    let whites = white_count(octaves);
    if !(0.0..whites as f32).contains(&x) || !(0.0..=1.0).contains(&y) {
        return None;
    }
    if y < BLACK_HEIGHT {
        if let Some((offset, _)) = black_keys(octaves).find(|&(_, centre)| (x - centre).abs() < BLACK_WIDTH / 2.0) {
            return Some(offset);
        }
    }
    Some(white_offset(x as usize))
}

/// Computer-keyboard keys laid out like a piano (`A` = C, `W` = C#, … `P` = D# above).
pub const TYPING: [char; 16] = ['a', 'w', 's', 'e', 'd', 'f', 't', 'g', 'y', 'h', 'u', 'j', 'k', 'o', 'l', 'p'];

/// Semitone offset of a typed key, if it plays a note.
pub fn typed_offset(key: char) -> Option<u8> {
    TYPING.iter().position(|&k| k == key.to_ascii_lowercase()).map(|i| i as u8)
}

/// Lowest note shown for an octave number (C4 = 60), kept within MIDI range for `octaves`.
pub fn base_note(octave: i32, octaves: usize) -> u8 {
    let top = 127 - 12 * octaves as i32;
    (12 * (octave + 1)).clamp(0, top - top % 12) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_semitones() {
        assert_eq!(white_count(2), 15);
        assert_eq!((0..8).map(white_offset).collect::<Vec<_>>(), [0, 2, 4, 5, 7, 9, 11, 12]);
        assert_eq!(black_keys(1).map(|(o, _)| o).collect::<Vec<_>>(), [1, 3, 6, 8, 10]);
        assert_eq!(key_at(2, 0.5, 0.9), Some(0), "bottom of the first white key is C");
        assert_eq!(key_at(2, 1.0, 0.3), Some(1), "between C and D near the top is C#");
        assert_eq!(key_at(2, 1.0, 0.8), Some(2), "below the black key falls through to a white key");
        assert_eq!(key_at(2, 3.0, 0.3), Some(5), "no black key between E and F");
        assert_eq!(key_at(2, 14.5, 0.5), Some(24), "closing C");
        assert_eq!(key_at(2, 15.0, 0.5), None);
    }

    #[test]
    fn typing_and_octaves() {
        assert_eq!(typed_offset('A'), Some(0));
        assert_eq!(typed_offset('k'), Some(12));
        assert_eq!(typed_offset('z'), None);
        assert_eq!(base_note(3, 4), 48);
        assert_eq!(base_note(-5, 4), 0);
        assert!(base_note(20, 4) as usize + 12 * 4 <= 127);
    }
}
