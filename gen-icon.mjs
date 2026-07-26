// Generates a 1024x1024 source PNG for the app icon (brass rounded square with a
// spark-orange bezel). No image libs — hand-rolls a PNG via zlib. Run once:
//   node gen-icon.mjs   → app-icon.png   (then: npx tauri icon app-icon.png)
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const S = 1024;
const buf = Buffer.alloc(S * S * 4);

const hex = (h) => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
const deep = hex("#0f1620");
const brass = hex("#b07a33");
const brassHi = hex("#e8c679");
const spark = hex("#f28c28");

function px(x, y, [r, g, b], a = 255) {
  const i = (y * S + x) * 4;
  buf[i] = r; buf[i + 1] = g; buf[i + 2] = b; buf[i + 3] = a;
}

const R = 180;                 // corner radius
const inset = 70;              // margin
const inRounded = (x, y, pad) => {
  const lo = inset + pad, hi = S - inset - pad;
  if (x < lo || x > hi || y < lo || y > hi) return false;
  const rr = R - pad;
  const cx = Math.min(Math.max(x, lo + rr), hi - rr);
  const cy = Math.min(Math.max(y, lo + rr), hi - rr);
  return (x - cx) ** 2 + (y - cy) ** 2 <= rr * rr;
};

for (let y = 0; y < S; y++) {
  for (let x = 0; x < S; x++) {
    if (inRounded(x, y, 26)) {
      // vertical brass gradient
      const t = (y - inset) / (S - 2 * inset);
      const mix = (a, b) => Math.round(a + (b - a) * t);
      px(x, y, [mix(brassHi[0], brass[0]), mix(brassHi[1], brass[1]), mix(brassHi[2], brass[2])]);
    } else if (inRounded(x, y, 0)) {
      px(x, y, spark);          // bezel
    } else {
      px(x, y, deep, 0);        // transparent background
    }
  }
}

// central "uplink" glyph: an upward chevron stack (spark-ink on brass)
const ink = hex("#1a1206");
function bar(cy, halfW, thick) {
  for (let dy = -thick; dy <= thick; dy++) {
    for (let dx = -halfW; dx <= halfW; dx++) {
      const x = S / 2 + dx;
      const y = cy + Math.abs(dx) * 0.55 + dy;   // chevron slope
      if (inRounded(x, y | 0, 40)) px(x | 0, y | 0, ink);
    }
  }
}
bar(560, 230, 26);
bar(680, 230, 26);

// PNG assembly
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td) >>> 0);
  return Buffer.concat([len, td, crc]);
}
function crc32(b) {
  let c = ~0;
  for (let i = 0; i < b.length; i++) {
    c ^= b[i];
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return ~c;
}
const raw = Buffer.alloc((S * 4 + 1) * S);
for (let y = 0; y < S; y++) {
  raw[y * (S * 4 + 1)] = 0;
  buf.copy(raw, y * (S * 4 + 1) + 1, y * S * 4, (y + 1) * S * 4);
}
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0); ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8; ihdr[9] = 6; // 8-bit RGBA
const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw)),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync("app-icon.png", png);
console.log("wrote app-icon.png", png.length, "bytes");
