// Generates the source icon for `tauri icon`.
// Three vertical bars = lanes; the leading one carries the accent.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";

const SIZE = 512;
const BG = [11, 11, 11, 255];
const ACCENT = [224, 49, 39, 255];
const PALE = [233, 231, 228, 255];
const MUTED = [58, 58, 58, 255];

const px = Buffer.alloc(SIZE * SIZE * 4);
const put = (x, y, [r, g, b, a]) => {
  const i = (y * SIZE + x) * 4;
  px[i] = r;
  px[i + 1] = g;
  px[i + 2] = b;
  px[i + 3] = a;
};

for (let y = 0; y < SIZE; y++) for (let x = 0; x < SIZE; x++) put(x, y, BG);

// Bars: x offsets, height fraction, colour.
const bars = [
  { x: 132, h: 0.62, c: ACCENT },
  { x: 232, h: 0.82, c: PALE },
  { x: 332, h: 0.46, c: MUTED },
];
const W = 48;
for (const bar of bars) {
  const height = Math.round(SIZE * 0.72 * bar.h);
  const bottom = Math.round(SIZE * 0.82);
  for (let y = bottom - height; y < bottom; y++) {
    for (let x = bar.x; x < bar.x + W; x++) put(x, y, bar.c);
  }
}

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
