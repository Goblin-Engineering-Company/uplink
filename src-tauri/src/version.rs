//! Segment-wise numeric compare of dotted build stamps (YYYY.MM.DD.N). Mirrors
//! `versionCompare` in website/lib/uplink.ts. NEVER string-compare build stamps.

use std::cmp::Ordering;

pub fn version_compare(a: &str, b: &str) -> Ordering {
    let pa: Vec<i64> = a.split('.').map(|n| n.parse().unwrap_or(0)).collect();
    let pb: Vec<i64> = b.split('.').map(|n| n.parse().unwrap_or(0)).collect();
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    Ordering::Equal
}

/// True when `latest` is strictly newer than `installed`.
pub fn is_newer(latest: &str, installed: &str) -> bool {
    version_compare(latest, installed) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_not_string() {
        // "2026.07.10.2" vs "2026.07.9.1": 10 > 9 numerically even though "10" < "9" as strings
        assert!(is_newer("2026.07.10.2", "2026.07.9.1"));
        assert!(!is_newer("2026.07.05.5", "2026.07.05.5"));
        assert!(is_newer("2026.07.12.1", "2026.07.10.2"));
    }
}
