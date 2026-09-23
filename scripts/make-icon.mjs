// Generates the source icon for `tauri icon`.
//
// The Divixi mark, as in ui/src/lib/Mark.svelte: four staves and an X
// across them, on the near-black tile. Same 64-unit geometry, scaled, so
// the taskbar icon and the rail mark are one drawing.
//
//   node scripts/make-icon.mjs && npx tauri icon scripts/out/icon-source.png
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";

const SIZE = 512;
const BG = [11, 11, 11, 255];
const ACCENT = [224, 49, 39, 255];
const PALE = [233, 231, 228, 255];

const px = Buffer.alloc(SIZE * SIZE * 4);
const put = (x, y, [r, g, b, a]) => {
  const i = (y * SIZE + x) * 4;
  px[i] = r;
  px[i + 1] = g;
  px[i + 2] = b;
  px[i + 3] = a;
};
const get = (x, y) => {
  const i = (y * SIZE + x) * 4;
  return [px[i], px[i + 1], px[i + 2], px[i + 3]];
};
/** Blend `c` over the pixel with coverage `a` in 0..1. */
const blend = (x, y, c, a) => {
  if (a <= 0) return;
  if (a >= 1) return put(x, y, c);
  const [r, g, b] = get(x, y);
  put(x, y, [Math.round(r + (c[0] - r) * a), Math.round(g + (c[1] - g) * a), Math.round(b + (c[2] - b) * a), 255]);
};

for (let y = 0; y < SIZE; y++) for (let x = 0; x < SIZE; x++) put(x, y, BG);

// Mark coordinates (64-unit box) to pixels.
const S = SIZE / 64;

// Axis-aligned rectangle.
const rect = (x, y, w, h, c) => {
  const x0 = Math.round(x * S), y0 = Math.round(y * S);
  const x1 = Math.round((x + w) * S), y1 = Math.round((y + h) * S);
  for (let yy = y0; yy < y1; yy++) for (let xx = x0; xx < x1; xx++) put(xx, yy, c);
};

// A stroke from (ax, ay) to (bx, by) of width w with square caps, antialiased
// by distance to the segment (one pixel of ramp).
const stroke = (ax, ay, bx, by, w, c) => {
  const [px0, py0, px1, py1] = [ax * S, ay * S, bx * S, by * S];
  const half = (w * S) / 2;
  const dx = px1 - px0, dy = py1 - py0;
  const len2 = dx * dx + dy * dy;
  // Square caps: extend the segment by half the width at both ends.
  const ext = half / Math.sqrt(len2);
  const ex0 = px0 - dx * ext, ey0 = py0 - dy * ext, ex1 = px1 + dx * ext, ey1 = py1 + dy * ext;
  const edx = ex1 - ex0, edy = ey1 - ey0, elen2 = edx * edx + edy * edy;
  const minX = Math.max(0, Math.floor(Math.min(ex0, ex1) - half - 1));
  const maxX = Math.min(SIZE - 1, Math.ceil(Math.max(ex0, ex1) + half + 1));
  const minY = Math.max(0, Math.floor(Math.min(ey0, ey1) - half - 1));
  const maxY = Math.min(SIZE - 1, Math.ceil(Math.max(ey0, ey1) + half + 1));
  for (let y = minY; y <= maxY; y++) {
    for (let x = minX; x <= maxX; x++) {
      const cx = x + 0.5, cy = y + 0.5;
      let t = ((cx - ex0) * edx + (cy - ey0) * edy) / elen2;
      t = Math.max(0, Math.min(1, t));
      const qx = ex0 + edx * t, qy = ey0 + edy * t;
      const d = Math.hypot(cx - qx, cy - qy);
      blend(x, y, c, Math.max(0, Math.min(1, half - d + 0.5)));
    }
  }
};

// The staves. Slightly heavier than the UI hairline so they survive 32px.
for (const y of [22, 30, 38, 46]) rect(8, y - 1, 48, 2, PALE);
// The X: the beat across the staves.
stroke(20, 12, 44, 56, 4, ACCENT);
stroke(44, 12, 20, 56, 4, ACCENT);

// Raw scanlines, each prefixed with filter type 0.
const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y++) {
  raw[y * (SIZE * 4 + 1)] = 0;
  px.copy(raw, y * (SIZE * 4 + 1) + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};

const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
};

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // RGBA
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

mkdirSync("scripts/out", { recursive: true });
writeFileSync("scripts/out/icon-source.png", png);
console.log(`wrote scripts/out/icon-source.png (${png.length} bytes)`);
