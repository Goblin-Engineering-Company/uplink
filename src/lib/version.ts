// Segment-wise numeric compare of dotted build stamps (YYYY.MM.DD.N). Mirrors
// versionCompare in website/lib/uplink.ts and the Rust core. Returns >0 when a
// is newer than b. NEVER string-compare these.
export function versionCompare(a: string, b: string): number {
  const pa = a.split(".").map((n) => parseInt(n, 10) || 0);
  const pb = b.split(".").map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}
