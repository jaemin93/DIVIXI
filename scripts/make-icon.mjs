// Generates the source icon for `tauri icon`.
//
// The membrane mark, as in ui/src/lib/Mark.svelte: four lanes below a
// hairline, one accent square above it. Same 64-unit geometry, scaled.
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

for (let y = 0; y < SIZE; y++) for (let x = 0; x < SIZE; x++) put(x, y, BG);

// Axis-aligned rectangle in 64-unit mark coordinates, scaled to SIZE.
const S = SIZE / 64;
const rect = (x, y, w, h, c) => {
  const x0 = Math.round(x * S), y0 = Math.round(y * S);
  const x1 = Math.round((x + w) * S), y1 = Math.round((y + h) * S);
  for (let yy = y0; yy < y1; yy++) for (let xx = x0; xx < x1; xx++) put(xx, yy, c);
};

// What crossed.
rect(29.5, 15, 5, 5, ACCENT);
// The membrane. Slightly heavier than the UI hairline so it survives 32px.
rect(8, 30, 48, 2, PALE);
// The lanes.
const lanes = 4;
for (let i = 0; i < lanes; i++) {
  const cx = 14 + (36 * i) / (lanes - 1);
  rect(cx - 1.75, 38, 3.5, 18, PALE);
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
