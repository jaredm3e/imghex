//! Byte-pattern search over the file, plus query parsing for the GUI's find bar.

/// Parse a hex query like `"DE AD BE EF"`, `"deadbeef"`, or `"0xDEAD"` into
/// bytes. Whitespace and a leading `0x` are ignored; requires an even number of
/// hex digits. Returns `None` on invalid input or an empty result.
pub fn parse_hex(query: &str) -> Option<Vec<u8>> {
    let cleaned: String = query
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if cleaned.is_empty() || !cleaned.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    let bytes = cleaned.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

/// All start offsets where `needle` occurs in `haystack` (overlapping matches
/// included). An empty needle matches nothing.
pub fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for start in 0..=(haystack.len() - needle.len()) {
        if &haystack[start..start + needle.len()] == needle {
            hits.push(start);
        }
    }
    hits
}

/// The first match at or after `from`, wrapping around to the start if needed.
pub fn find_next(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let last_start = haystack.len() - needle.len();
    let start = from.min(last_start + 1);
    for s in start..=last_start {
        if &haystack[s..s + needle.len()] == needle {
            return Some(s);
        }
    }
    // Wrap around.
    for s in 0..start.min(last_start + 1) {
        if &haystack[s..s + needle.len()] == needle {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_variants() {
        assert_eq!(parse_hex("DEADBEEF"), Some(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(parse_hex("de ad be ef"), Some(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(parse_hex("0x42"), Some(vec![0x42]));
        assert_eq!(parse_hex(""), None);
        assert_eq!(parse_hex("abc"), None); // odd length
        assert_eq!(parse_hex("zz"), None); // not hex
    }

    #[test]
    fn finds_all_occurrences() {
        let hay = b"BMxxBMyyBM";
        assert_eq!(find_all(hay, b"BM"), vec![0, 4, 8]);
        assert_eq!(find_all(hay, b"zz"), Vec::<usize>::new());
        assert_eq!(find_all(hay, b""), Vec::<usize>::new());
    }

    #[test]
    fn finds_overlapping() {
        assert_eq!(find_all(b"aaaa", b"aa"), vec![0, 1, 2]);
    }

    #[test]
    fn find_next_wraps_around() {
        let hay = b"BMxxBM";
        assert_eq!(find_next(hay, b"BM", 0), Some(0));
        assert_eq!(find_next(hay, b"BM", 1), Some(4));
        assert_eq!(find_next(hay, b"BM", 5), Some(0)); // wrapped
        assert_eq!(find_next(hay, b"zz", 0), None);
    }
}
