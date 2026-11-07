//! Value formatting and bubble text fitting and merging

use crate::decode::Radix;

/// Ascii: `\0 \t \n \r` escaped, other non-printables as `.`;
/// printable is 0x20–0x7E or ≥ 0xA0.
pub fn ascii_char(b: u8) -> String {
    match b {
        0 => "\\0".into(),
        b'\t' => "\\t".into(),
        b'\n' => "\\n".into(),
        b'\r' => "\\r".into(),
        0x20..=0x7E => (b as char).to_string(),
        0xA0..=0xFF => char::from(b).to_string(),
        _ => ".".into(),
    }
}

/// Byte arrays.
pub fn format_bytes(bytes: &[u8], radix: Radix) -> String {
    match radix {
        Radix::Hex => format!("0x{}", bytes.iter().map(|b| format!("{b:02X}")).collect::<String>()),
        Radix::Dec => bytes.iter().map(|&b| (b as i8).to_string()).collect::<Vec<_>>().join(" "),
        Radix::Bin => format!("0b{}", bytes.iter().map(|b| format!("{b:08b}")).collect::<String>()),
        Radix::Ascii => bytes.iter().map(|&b| ascii_char(b)).collect(),
        Radix::Native => bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
    }
}

/// Integers of `bits` width (8 for byte-sized decoder values, 64
/// for 64-bit ones).
pub fn format_int(v: u64, bits: u32, radix: Radix) -> String {
    let hex_digits = bits.div_ceil(4) as usize;
    match radix {
        Radix::Hex => format!("0x{v:0hex_digits$X}"),
        Radix::Dec => v.to_string(),
        Radix::Bin => format!("0b{v:0width$b}", width = bits as usize),
        Radix::Ascii if bits <= 8 => format!("'{}'", ascii_char(v as u8)),
        Radix::Ascii => v.to_string(),
        // As decoders print values.
        Radix::Native => format!("{v:02X}"),
    }
}

/// The longest string whose width fits `width_px` (each character
/// `char_px` wide), else the shortest, or nothing if even that doesn't fit.
pub fn fit_text(texts: &[String], width_px: f32, char_px: f32) -> Option<&str> {
    let fits = |t: &String| t.chars().count() as f32 * char_px <= width_px;
    texts.iter().find(|t| fits(t)).map(String::as_str)
}

/// Merge thresholds.
pub const MERGE_MAX_WIDTH: f32 = 30.0;
pub const MERGE_MAX_GAP: f32 = 15.0;

/// A bubble or a merged multi-bubble: frames `first..=last` of the input.
#[derive(Clone, Debug, PartialEq)]
pub struct BubbleGroup {
    pub first: usize,
    pub last: usize,
    pub x0: f32,
    pub x1: f32,
}

impl BubbleGroup {
    pub fn count(&self) -> usize {
        self.last - self.first + 1
    }
}

/// Consecutive bubbles (x ranges, sorted) merge when each is
/// ≤ 30 px wide and they are ≤ 15 px apart.
pub fn merge_bubbles(xs: &[(f32, f32)]) -> Vec<BubbleGroup> {
    let mut out: Vec<BubbleGroup> = Vec::new();
    for (i, &(x0, x1)) in xs.iter().enumerate() {
        let narrow = x1 - x0 <= MERGE_MAX_WIDTH;
        if let Some(g) = out.last_mut() {
            let prev = xs[g.last];
            let prev_narrow = prev.1 - prev.0 <= MERGE_MAX_WIDTH;
            if narrow && prev_narrow && x0 - prev.1 <= MERGE_MAX_GAP {
                g.last = i;
                g.x1 = g.x1.max(x1);
                continue;
            }
        }
        out.push(BubbleGroup { first: i, last: i, x0, x1 });
    }
    out
}

/// multi-bubble text: short strings joined by `·` (nothing for
/// Ascii), at most 200 characters with `…`.
pub fn multi_text<'a>(shorts: impl Iterator<Item = &'a str>, radix: Radix) -> String {
    let sep = if radix == Radix::Ascii { "" } else { "·" };
    let mut s = String::new();
    for (i, t) in shorts.enumerate() {
        if i > 0 {
            s.push_str(sep);
        }
        // `'A'` from raw values: spelled without the quotes.
        let t = match radix {
            Radix::Ascii => {
                t.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')).filter(|t| !t.is_empty()).unwrap_or(t)
            }
            _ => t,
        };
        s.push_str(t);
        if s.chars().count() > 200 {
            let cut: String = s.chars().take(199).collect();
            return format!("{cut}…");
        }
    }
    s
}

/// Count badge.
pub fn count_badge(n: usize) -> String {
    crate::core::format::count_capped(n as u64)
}

/// `texts` with a trailing hex number equal to `value` (as
/// decoders print it, `%02X`) replaced by `value` in `radix`. Texts
/// without it are kept.
pub fn apply_radix(texts: &[String], value: u64, bits: u32, radix: Radix) -> Vec<String> {
    if radix == Radix::Native {
        return texts.to_vec();
    }
    let hex = format!("{value:02X}");
    let new = format_int(value, bits, radix);
    texts
        .iter()
        .map(|t| {
            let at = t.len().wrapping_sub(hex.len());
            let ends = t.len() >= hex.len() && t.is_char_boundary(at) && t[at..].eq_ignore_ascii_case(&hex);
            // A whole number: nothing alphanumeric right before it.
            let whole = ends && !t[..at].ends_with(|c: char| c.is_ascii_alphanumeric());
            if whole { format!("{}{new}", &t[..at]) } else { t.clone() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U-12: radix formatting of bytes and integers.
    #[test]
    fn u12_radix_formatting() {
        let b = [0x48, 0x00, 0x0A, 0xFF];
        assert_eq!(format_bytes(&b, Radix::Hex), "0x48000AFF");
        assert_eq!(format_bytes(&b, Radix::Dec), "72 0 10 -1");
        assert_eq!(format_bytes(&b, Radix::Bin), "0b01001000000000000000101011111111");
        assert_eq!(format_bytes(&b, Radix::Ascii), "H\\0\\nÿ");
        assert_eq!(format_bytes(&[0x07, b'\t', b'\r', 0x7F], Radix::Ascii), ".\\t\\r.");
        assert_eq!(format_int(0x41, 8, Radix::Hex), "0x41");
        assert_eq!(format_int(0x41, 64, Radix::Hex), "0x0000000000000041");
        assert_eq!(format_int(5, 8, Radix::Bin), "0b00000101");
        assert_eq!(format_int(65, 8, Radix::Dec), "65");
        assert_eq!(format_int(65, 8, Radix::Ascii), "'A'");
        assert_eq!(format_int(7, 8, Radix::Ascii), "'.'");
    }

    /// Decoder texts with the value in the radix.
    #[test]
    fn radix_replaces_the_trailing_value() {
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let i2c = t(&["Data write: 3A", "DW: 3A", "3A"]);
        assert_eq!(apply_radix(&i2c, 0x3A, 8, Radix::Hex), t(&["Data write: 0x3A", "DW: 0x3A", "0x3A"]));
        assert_eq!(apply_radix(&i2c, 0x3A, 8, Radix::Dec), t(&["Data write: 58", "DW: 58", "58"]));
        assert_eq!(apply_radix(&i2c, 0x3A, 8, Radix::Ascii)[2], "':'");
        assert_eq!(apply_radix(&i2c, 0x3A, 8, Radix::Native), i2c);
        // Not the value, or part of a longer word: unchanged.
        assert_eq!(apply_radix(&t(&["Warning", "13A", "ä"]), 0x3A, 8, Radix::Dec), t(&["Warning", "13A", "ä"]));
        // A 16-bit SPI word.
        assert_eq!(apply_radix(&t(&["1234"]), 0x1234, 16, Radix::Bin), t(&["0b0001001000110100"]));
    }

    /// U-11: text selection by width; merge rules; `99+`.
    #[test]
    fn u11_bubble_text_and_merging() {
        let texts: Vec<String> = ["Data: 0x41 'A'", "0x41", "41"].iter().map(|s| s.to_string()).collect();
        assert_eq!(fit_text(&texts, 100.0, 6.5), Some("Data: 0x41 'A'"));
        assert_eq!(fit_text(&texts, 30.0, 6.5), Some("0x41"));
        assert_eq!(fit_text(&texts, 13.0, 6.5), Some("41"));
        assert_eq!(fit_text(&texts, 10.0, 6.5), None);

        // Narrow and close: merged; a wide bubble breaks the run.
        let xs = [(0.0, 20.0), (30.0, 50.0), (60.0, 70.0), (80.0, 200.0), (210.0, 220.0), (240.0, 250.0)];
        let g = merge_bubbles(&xs);
        let spans: Vec<(usize, usize)> = g.iter().map(|g| (g.first, g.last)).collect();
        assert_eq!(spans, [(0, 2), (3, 3), (4, 4), (5, 5)], "a 20 px gap doesn't merge");
        assert_eq!(g[0].count(), 3);
        assert_eq!((g[0].x0, g[0].x1), (0.0, 70.0));

        assert_eq!(multi_text(["48", "65", "6C"].into_iter(), Radix::Hex), "48·65·6C");
        assert_eq!(multi_text(["H", "e"].into_iter(), Radix::Ascii), "He");
        let long = multi_text(std::iter::repeat_n("AB", 150), Radix::Ascii);
        assert_eq!(long.chars().count(), 200);
        assert!(long.ends_with('…'));
        assert_eq!((count_badge(12), count_badge(100)), ("12".to_owned(), "99+".to_owned()));
    }
}
