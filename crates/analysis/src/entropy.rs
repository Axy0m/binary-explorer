//! Shannon entropy across a file — the "byte entropy" view.
//!
//! High entropy (near 1.0) means the bytes look random: compressed, encrypted,
//! or already-packed data. Low entropy means structure or repetition. Plotting
//! it across the file quickly reveals where the interesting/packed regions are.

/// Normalized Shannon entropy (0.0–1.0) of each of `buckets` equal-width slices
/// of `bytes`. 8 bits of entropy (a perfectly uniform byte distribution) maps to
/// 1.0. Returns fewer buckets than requested only when the input is shorter than
/// `buckets` bytes.
pub fn entropy(bytes: &[u8], buckets: usize) -> Vec<f32> {
    if bytes.is_empty() || buckets == 0 {
        return Vec::new();
    }
    let n = buckets.min(bytes.len());
    let chunk = bytes.len() / n;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = i * chunk;
        // The last bucket absorbs any remainder so every byte is counted.
        let end = if i == n - 1 { bytes.len() } else { start + chunk };
        out.push(shannon(&bytes[start..end]));
    }
    out
}

/// Normalized Shannon entropy of a single slice, in [0.0, 1.0].
fn shannon(slice: &[u8]) -> f32 {
    if slice.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &b in slice {
        counts[b as usize] += 1;
    }
    let len = slice.len() as f64;
    let mut h = 0.0f64;
    for &c in counts.iter() {
        if c > 0 {
            let p = c as f64 / len;
            h -= p * p.log2();
        }
    }
    (h / 8.0) as f32 // 8 bits max -> normalize to 0..1
}
