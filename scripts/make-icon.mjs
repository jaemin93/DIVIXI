// Draws the app icon: the source for `tauri icon`, and every desktop size
// (the PNGs in src-tauri/icons and icon.ico) drawn at its own pixel size.
//
// The Divixi mark, as in ui/src/lib/Mark.svelte: four staves and an X
// across them, on a transparent background. Same 64-unit geometry, scaled, so
// the taskbar icon and the rail mark are one drawing.
//
// Why each size is drawn, not shrunk from 512: at 16–32 px a stave is about
// one pixel, and shrinking spreads it over two half-transparent rows, so the
// taskbar shows the ochre washed out to grey. Drawn per size, the staves sit
// on whole pixels at full colour, as the rail draws them (crispEdges).
//
//   node scripts/make-icon.mjs
//
// `npx tauri icon scripts/out/icon-source.png` (icns, mobile) overwrites the
// desktop sizes with shrunk ones: run this script again after it.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";

const ACCENT = [224, 49, 39, 255];
const STAVE = [168, 132, 61, 255];

/** The mark drawn on an N×N transparent canvas, RGBA. */
function render(N) {
  const px = Buffer.alloc(N * N * 4);
  const put = (x, y, [r, g, b, a]) => {
    const i = (y * N + x) * 4;
    px[i] = r;
    px[i + 1] = g;
    px[i + 2] = b;
    px[i + 3] = a;
  };
  const get = (x, y) => {
    const i = (y * N + x) * 4;
    return [px[i], px[i + 1], px[i + 2], px[i + 3]];
  };
  /** Composite `c` over the pixel with coverage `a` in 0..1 (the pixel may be transparent). */
  const blend = (x, y, c, a) => {
    if (a <= 0) return;
    if (a >= 1) return put(x, y, c);
    const [r, g, b, pa] = get(x, y);
    const da = pa / 255;
    const oa = a + da * (1 - a);
    if (oa <= 0) return;
    const mix = (top, under) => Math.round((top * a + under * da * (1 - a)) / oa);
    put(x, y, [mix(c[0], r), mix(c[1], g), mix(c[2], b), Math.round(oa * 255)]);
  };

  // Mark coordinates (64-unit box) to pixels.
  const S = N / 64;

  // The staves: whole pixel rows at full colour, at least one pixel thick
  // (two units, slightly heavier than the UI hairline, where there is room).
  const t = Math.max(1, Math.round(2 * S));
  const x0 = Math.round(8 * S);
  const x1 = Math.round(56 * S);
  for (const y of [22, 30, 38, 46]) {
    const y0 = Math.round(y * S - t / 2);
    for (let yy = y0; yy < y0 + t; yy++) for (let xx = x0; xx < x1; xx++) put(xx, yy, STAVE);
  }

  // A stroke from (ax, ay) to (bx, by) of width w with square caps, antialiased
  // by distance to the segment (one pixel of ramp).
  const stroke = (ax, ay, bx, by, w, c) => {
    const [px0, py0, px1, py1] = [ax * S, ay * S, bx * S, by * S];
    const half = Math.max(0.5, (w * S) / 2);
    const dx = px1 - px0,
      dy = py1 - py0;
    const len2 = dx * dx + dy * dy;
    // Square caps: extend the segment by half the width at both ends.
    const ext = half / Math.sqrt(len2);
    const ex0 = px0 - dx * ext,
      ey0 = py0 - dy * ext,
      ex1 = px1 + dx * ext,
      ey1 = py1 + dy * ext;
    const edx = ex1 - ex0,
      edy = ey1 - ey0,
      elen2 = edx * edx + edy * edy;
    const minX = Math.max(0, Math.floor(Math.min(ex0, ex1) - half - 1));
    const maxX = Math.min(N - 1, Math.ceil(Math.max(ex0, ex1) + half + 1));
    const minY = Math.max(0, Math.floor(Math.min(ey0, ey1) - half - 1));
    const maxY = Math.min(N - 1, Math.ceil(Math.max(ey0, ey1) + half + 1));
    for (let y = minY; y <= maxY; y++) {
      for (let x = minX; x <= maxX; x++) {
        const cx = x + 0.5,
          cy = y + 0.5;
        let k = ((cx - ex0) * edx + (cy - ey0) * edy) / elen2;
        k = Math.max(0, Math.min(1, k));
        const qx = ex0 + edx * k,
          qy = ey0 + edy * k;
        const d = Math.hypot(cx - qx, cy - qy);
        blend(x, y, c, Math.max(0, Math.min(1, half - d + 0.5)));
      }
    }
  };

  // The X: the beat across the staves.
  stroke(20, 12, 44, 56, 4, ACCENT);
  stroke(44, 12, 20, 56, 4, ACCENT);
  return px;
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

/** An N×N RGBA PNG of the mark. */
function png(N) {
  const px = render(N);
  // Raw scanlines, each prefixed with filter type 0.
  const raw = Buffer.alloc(N * (N * 4 + 1));
  for (let y = 0; y < N; y++) {
    raw[y * (N * 4 + 1)] = 0;
    px.copy(raw, y * (N * 4 + 1) + 1, y * N * 4, (y + 1) * N * 4);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(N, 0);
  ihdr.writeUInt32BE(N, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** A Windows .ico holding one PNG per size. */
function ico(sizes) {
  const images = sizes.map(png);
  const head = Buffer.alloc(6 + 16 * sizes.length);
  head.writeUInt16LE(0, 0);
  head.writeUInt16LE(1, 2); // icon
  head.writeUInt16LE(sizes.length, 4);
  let at = head.length;
  sizes.forEach((n, i) => {
    const e = 6 + i * 16;
    head[e] = n >= 256 ? 0 : n;
    head[e + 1] = n >= 256 ? 0 : n;
    head.writeUInt16LE(1, e + 4); // planes
    head.writeUInt16LE(32, e + 6); // bits per pixel
    head.writeUInt32LE(images[i].length, e + 8);
    head.writeUInt32LE(at, e + 12);
    at += images[i].length;
  });
  return Buffer.concat([head, ...images]);
}

mkdirSync("scripts/out", { recursive: true });
writeFileSync("scripts/out/icon-source.png", png(512));

const ICONS = "src-tauri/icons";
const desktop = {
  "32x32.png": 32,
  "64x64.png": 64,
  "128x128.png": 128,
  "128x128@2x.png": 256,
  "icon.png": 512,
  "Square30x30Logo.png": 30,
  "Square44x44Logo.png": 44,
  "Square71x71Logo.png": 71,
  "Square89x89Logo.png": 89,
  "Square107x107Logo.png": 107,
  "Square142x142Logo.png": 142,
  "Square150x150Logo.png": 150,
  "Square284x284Logo.png": 284,
  "Square310x310Logo.png": 310,
  "StoreLogo.png": 50,
};
for (const [name, n] of Object.entries(desktop)) writeFileSync(`${ICONS}/${name}`, png(n));
// The taskbar picks from these: every size Windows asks for at common scales.
writeFileSync(`${ICONS}/icon.ico`, ico([16, 20, 24, 32, 40, 48, 64, 256]));
console.log(`wrote scripts/out/icon-source.png, ${Object.keys(desktop).length} PNGs and icon.ico in ${ICONS}`);
