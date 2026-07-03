//! Statistics over an arbitrary byte range, for the multi-select inspector.
//!
//! Pure and GUI-agnostic: [`ByteStats::compute`] turns a slice of bytes into a
//! histogram plus summary figures (min/max/mean, distinct values, Shannon
//! entropy). The frontend renders the histogram; this module just counts.

/// Summary statistics for a range of bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct ByteStats {
    pub count: usize,
    pub min: u8,
    pub max: u8,
    pub sum: u64,
    pub mean: f64,
    /// Number of distinct byte values present (1..=256).
    pub distinct: usize,
    /// Shannon entropy in bits per byte (0.0 ..= 8.0).
    pub entropy: f64,
    /// Count of each byte value 0..=255.
    pub histogram: [u32; 256],
    /// The most frequently occurring byte value.
    pub most_common: u8,
    /// How many times `most_common` occurs.
    pub most_common_count: u32,
}

impl ByteStats {
    /// Compute statistics for `bytes`. Returns `None` for an empty slice.
    pub fn compute(bytes: &[u8]) -> Option<ByteStats> {
        if bytes.is_empty() {
            return None;
        }
        let mut histogram = [0u32; 256];
        let mut sum = 0u64;
        let mut min = u8::MAX;
        let mut max = u8::MIN;
        for &b in bytes {
            histogram[b as usize] += 1;
            sum += b as u64;
            min = min.min(b);
            max = max.max(b);
        }

        let count = bytes.len();
        let mean = sum as f64 / count as f64;

        let mut distinct = 0usize;
        let mut entropy = 0.0f64;
        let mut most_common = 0u8;
        let mut most_common_count = 0u32;
        for (value, &c) in histogram.iter().enumerate() {
            if c == 0 {
                continue;
            }
            distinct += 1;
            let p = c as f64 / count as f64;
            entropy -= p * p.log2();
            if c > most_common_count {
                most_common_count = c;
                most_common = value as u8;
            }
        }

        Some(ByteStats {
            count,
            min,
            max,
            sum,
            mean,
            distinct,
            entropy,
            histogram,
            most_common,
            most_common_count,
        })
    }
}

/// Shannon entropy (bits/byte, 0.0..=8.0) of a byte slice. 0 for empty input.
pub fn entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut histogram = [0u32; 256];
    for &b in bytes {
        histogram[b as usize] += 1;
    }
    let n = bytes.len() as f64;
    histogram
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Entropy of each consecutive `block_size`-byte block, for a whole-file
/// entropy strip. The final block may be shorter. Empty input → empty vec.
pub fn block_entropies(bytes: &[u8], block_size: usize) -> Vec<f64> {
    if bytes.is_empty() || block_size == 0 {
        return Vec::new();
    }
    bytes.chunks(block_size).map(entropy).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_none() {
        assert!(ByteStats::compute(&[]).is_none());
    }

    #[test]
    fn constant_data_has_zero_entropy() {
        let s = ByteStats::compute(&[7u8; 100]).unwrap();
        assert_eq!(s.count, 100);
        assert_eq!(s.min, 7);
        assert_eq!(s.max, 7);
        assert_eq!(s.distinct, 1);
        assert_eq!(s.entropy, 0.0);
        assert_eq!(s.most_common, 7);
        assert_eq!(s.most_common_count, 100);
        assert_eq!(s.histogram[7], 100);
    }

    #[test]
    fn two_equal_values_give_one_bit_of_entropy() {
        let s = ByteStats::compute(&[0, 255, 0, 255]).unwrap();
        assert_eq!(s.min, 0);
        assert_eq!(s.max, 255);
        assert_eq!(s.mean, 127.5);
        assert_eq!(s.distinct, 2);
        assert!((s.entropy - 1.0).abs() < 1e-9);
    }

    #[test]
    fn uniform_bytes_reach_max_entropy() {
        // Every value 0..=255 exactly once → 8 bits/byte.
        let data: Vec<u8> = (0..=255).collect();
        let s = ByteStats::compute(&data).unwrap();
        assert_eq!(s.distinct, 256);
        assert!((s.entropy - 8.0).abs() < 1e-9);
        assert_eq!(s.mean, 127.5);
    }

    #[test]
    fn block_entropies_split_and_measure() {
        // First block all-constant (0 bits), second block two values (1 bit).
        let mut data = vec![0u8; 4];
        data.extend_from_slice(&[0, 1, 0, 1]);
        let e = block_entropies(&data, 4);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], 0.0);
        assert!((e[1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn block_entropies_handles_short_final_block() {
        let e = block_entropies(&[0, 0, 0, 0, 0], 4);
        assert_eq!(e.len(), 2); // 4 + 1
    }
}
