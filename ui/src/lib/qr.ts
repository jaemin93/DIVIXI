/**
 * A QR code, as a grid of modules. Byte mode, error correction level M.
 *
 * Written here rather than taken as a dependency: the one thing divixi needs
 * to encode is a pairing URL of about 250 characters, the payload is a live
 * credential that must not travel through anything we did not write, and the
 * result is drawn as inline SVG so it takes the theme's colours and stays
 * crisp at any size.
 *
 * Level M (about 15% recovery) is the usual choice for a code read off a lit
 * screen from a hand's distance. Versions 1 to 20 are supported, which reaches
 * 666 bytes -- comfortably past the longest address a tailnet can produce.
 *
 * Deliberately knows nothing about what it is encoding, and never logs: the
 * payload carries a token.
 *
 * ISO/IEC 18004. The tables below are the standard's, for level M only.
 */

/** Error-correction level M, as the format information encodes it. */
const EC_LEVEL_M = 0b00;

/**
 * Per version: error-correction codewords per block, then the two groups as
 * (blocks, data codewords per block). Group two is absent for most versions.
 *
 * `totalCodewords` in the test cross-checks every row against the standard's
 * own total for that version, which catches a transcription slip in any of
 * the five numbers.
 */
type Blocks = { ecPerBlock: number; g1: number; g1Data: number; g2: number; g2Data: number };

const BLOCKS: Blocks[] = [
  { ecPerBlock: 10, g1: 1, g1Data: 16, g2: 0, g2Data: 0 }, // 1
  { ecPerBlock: 16, g1: 1, g1Data: 28, g2: 0, g2Data: 0 },
  { ecPerBlock: 26, g1: 1, g1Data: 44, g2: 0, g2Data: 0 },
  { ecPerBlock: 18, g1: 2, g1Data: 32, g2: 0, g2Data: 0 },
  { ecPerBlock: 24, g1: 2, g1Data: 43, g2: 0, g2Data: 0 }, // 5
  { ecPerBlock: 16, g1: 4, g1Data: 27, g2: 0, g2Data: 0 },
  { ecPerBlock: 18, g1: 4, g1Data: 31, g2: 0, g2Data: 0 },
  { ecPerBlock: 22, g1: 2, g1Data: 38, g2: 2, g2Data: 39 },
  { ecPerBlock: 22, g1: 3, g1Data: 36, g2: 2, g2Data: 37 },
  { ecPerBlock: 26, g1: 4, g1Data: 43, g2: 1, g2Data: 44 }, // 10
  { ecPerBlock: 30, g1: 1, g1Data: 50, g2: 4, g2Data: 51 },
  { ecPerBlock: 22, g1: 6, g1Data: 36, g2: 2, g2Data: 37 },
  { ecPerBlock: 22, g1: 8, g1Data: 37, g2: 1, g2Data: 38 },
  { ecPerBlock: 24, g1: 4, g1Data: 40, g2: 5, g2Data: 41 },
  { ecPerBlock: 24, g1: 5, g1Data: 41, g2: 5, g2Data: 42 }, // 15
  { ecPerBlock: 28, g1: 7, g1Data: 45, g2: 3, g2Data: 46 },
  { ecPerBlock: 28, g1: 10, g1Data: 46, g2: 1, g2Data: 47 },
  { ecPerBlock: 26, g1: 9, g1Data: 43, g2: 4, g2Data: 44 },
  { ecPerBlock: 26, g1: 3, g1Data: 44, g2: 11, g2Data: 45 },
  { ecPerBlock: 26, g1: 3, g1Data: 41, g2: 13, g2Data: 42 }, // 20
];

/** Alignment-pattern centre coordinates, by version. Version 1 has none. */
const ALIGNMENT: number[][] = [
  [],
  [6, 18],
  [6, 22],
  [6, 26],
  [6, 30],
  [6, 34],
  [6, 22, 38],
  [6, 24, 42],
  [6, 26, 46],
  [6, 28, 50],
  [6, 30, 54],
  [6, 32, 58],
  [6, 34, 62],
  [6, 26, 46, 66],
  [6, 26, 48, 70],
  [6, 26, 50, 74],
  [6, 30, 54, 78],
  [6, 30, 56, 82],
  [6, 30, 58, 86],
  [6, 34, 62, 90],
];

export const MAX_VERSION = BLOCKS.length;

/** The modules a version's grid is across: 21 for version 1, +4 each step. */
export const sizeOf = (version: number): number => 17 + 4 * version;

/** Data codewords a version holds at level M, across all its blocks. */
export function dataCapacity(version: number): number {
  const b = BLOCKS[version - 1];
  return b.g1 * b.g1Data + b.g2 * b.g2Data;
}

/** Every codeword a version holds, data and error correction together. */
export function totalCodewords(version: number): number {
  const b = BLOCKS[version - 1];
  return dataCapacity(version) + (b.g1 + b.g2) * b.ecPerBlock;
}

/**
 * Bytes a version can carry in byte mode: the data capacity, less the mode
 * indicator (4 bits) and the character count (8 bits below version 10, 16 at
 * and above).
 */
export function byteCapacity(version: number): number {
  const headerBits = 4 + (version < 10 ? 8 : 16);
  return dataCapacity(version) - Math.ceil(headerBits / 8);
}

// ----- GF(256) and Reed-Solomon -----
//
// The field QR uses: arithmetic modulo the primitive polynomial 0x11d.

const EXP = new Uint8Array(512);
const LOG = new Uint8Array(256);
{
  let x = 1;
  for (let i = 0; i < 255; i++) {
    EXP[i] = x;
    LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  for (let i = 255; i < 512; i++) EXP[i] = EXP[i - 255];
}

const mul = (a: number, b: number): number => (a === 0 || b === 0 ? 0 : EXP[LOG[a] + LOG[b]]);

/** The generator polynomial for `degree` error-correction codewords. */
function generator(degree: number): Uint8Array {
  let poly = new Uint8Array([1]);
  for (let d = 0; d < degree; d++) {
    const next = new Uint8Array(poly.length + 1);
    for (let i = 0; i < poly.length; i++) {
      next[i] ^= poly[i];
      next[i + 1] ^= mul(poly[i], EXP[d]);
    }
    poly = next;
  }
  return poly;
}

/**
 * The `degree` error-correction codewords for one block of data codewords:
 * the remainder of the data, shifted up, divided by the generator.
 */
export function errorCorrection(data: readonly number[] | Uint8Array, degree: number): number[] {
  const gen = generator(degree);
  const rem = new Uint8Array(data.length + degree);
  rem.set(data);
  for (let i = 0; i < data.length; i++) {
    const factor = rem[i];
    if (factor === 0) continue;
    for (let j = 0; j < gen.length; j++) rem[i + j] ^= mul(gen[j], factor);
  }
  return Array.from(rem.slice(data.length));
}

// ----- the bit stream -----

class Bits {
  readonly bits: number[] = [];
  push(value: number, length: number): void {
    for (let i = length - 1; i >= 0; i--) this.bits.push((value >>> i) & 1);
  }
}

/**
 * Data codewords for `bytes` at `version`: the byte-mode header, the data, a
 * terminator, and the standard alternating padding.
 */
function dataCodewords(bytes: Uint8Array, version: number): number[] {
  const capacity = dataCapacity(version) * 8;
  const b = new Bits();
  b.push(0b0100, 4); // byte mode
  b.push(bytes.length, version < 10 ? 8 : 16);
  for (const byte of bytes) b.push(byte, 8);
  // Terminator: up to four zero bits, fewer if the end is nearer than that.
  b.push(0, Math.min(4, capacity - b.bits.length));
  while (b.bits.length % 8 !== 0) b.bits.push(0);
  const words: number[] = [];
  for (let i = 0; i < b.bits.length; i += 8) {
    let w = 0;
    for (let j = 0; j < 8; j++) w = (w << 1) | b.bits[i + j];
    words.push(w);
  }
  // The standard's two pad codewords, alternating, to fill the version.
  for (let i = 0; words.length < dataCapacity(version); i++) words.push(i % 2 === 0 ? 0xec : 0x11);
  return words;
}

/**
 * Data and error-correction codewords, interleaved as the standard requires:
 * the first codeword of every block, then the second of every block, and so
 * on, data first and then error correction.
 */
function interleave(words: number[], version: number): number[] {
  const { ecPerBlock, g1, g1Data, g2, g2Data } = BLOCKS[version - 1];
  const blocks: number[][] = [];
  const ec: number[][] = [];
  let at = 0;
  for (let i = 0; i < g1 + g2; i++) {
    const size = i < g1 ? g1Data : g2Data;
    const block = words.slice(at, at + size);
    at += size;
    blocks.push(block);
    ec.push(errorCorrection(block, ecPerBlock));
  }
  const out: number[] = [];
  const longest = Math.max(g1Data, g2 > 0 ? g2Data : 0);
  for (let i = 0; i < longest; i++) {
    for (const block of blocks) if (i < block.length) out.push(block[i]);
  }
  for (let i = 0; i < ecPerBlock; i++) {
    for (const block of ec) out.push(block[i]);
  }
  return out;
}

// ----- the grid -----

/** `null` where nothing has been placed yet, so masking can skip the rest. */
type Grid = (boolean | null)[][];

function blank(size: number): Grid {
  return Array.from({ length: size }, () => Array.from({ length: size }, () => null as boolean | null));
}

function placeFinder(g: Grid, row: number, col: number): void {
  for (let r = -1; r <= 7; r++) {
    for (let c = -1; c <= 7; c++) {
      const y = row + r;
      const x = col + c;
      if (y < 0 || y >= g.length || x < 0 || x >= g.length) continue;
      // Outside the 7x7 pattern is the separator: always light. Running the
      // ring through the edge test below would draw the pattern's own border
      // one module out and bleed it into the separator.
      if (r < 0 || r > 6 || c < 0 || c > 6) {
        g[y][x] = false;
        continue;
      }
      const edge = r === 0 || r === 6 || c === 0 || c === 6;
      const core = r >= 2 && r <= 4 && c >= 2 && c <= 4;
      g[y][x] = edge || core;
    }
  }
}

function placeFunctions(g: Grid, version: number): void {
  const size = g.length;
  placeFinder(g, 0, 0);
  placeFinder(g, 0, size - 7);
  placeFinder(g, size - 7, 0);

  // Timing patterns, between the finders.
  for (let i = 8; i < size - 8; i++) {
    g[6][i] = i % 2 === 0;
    g[i][6] = i % 2 === 0;
  }

  // Alignment patterns, except where they would sit on a finder.
  const centres = ALIGNMENT[version - 1];
  for (const r of centres) {
    for (const c of centres) {
      const onFinder = (r <= 8 && c <= 8) || (r <= 8 && c >= size - 9) || (r >= size - 9 && c <= 8);
      if (onFinder) continue;
      for (let dr = -2; dr <= 2; dr++) {
        for (let dc = -2; dc <= 2; dc++) {
          g[r + dr][c + dc] = Math.max(Math.abs(dr), Math.abs(dc)) !== 1;
        }
      }
    }
  }

  // The one module that is always dark.
  g[size - 8][8] = true;

  // Reserve the format areas so data placement steps over them.
  for (let i = 0; i < 9; i++) {
    if (g[8][i] === null) g[8][i] = false;
    if (g[i][8] === null) g[i][8] = false;
  }
  for (let i = 0; i < 8; i++) {
    if (g[8][size - 1 - i] === null) g[8][size - 1 - i] = false;
    if (g[size - 1 - i][8] === null) g[size - 1 - i][8] = false;
  }

  // Version information, for version 7 and above: 6 data bits and 12 of BCH.
  if (version >= 7) {
    let rem = version;
    for (let i = 0; i < 12; i++) rem = (rem << 1) ^ ((rem >>> 11) * 0x1f25);
    const bits = ((version << 12) | rem) >>> 0;
    for (let i = 0; i < 18; i++) {
      const bit = ((bits >>> i) & 1) === 1;
      const a = Math.floor(i / 3);
      const b = (i % 3) + size - 11;
      g[b][a] = bit;
      g[a][b] = bit;
    }
  }
}

/** Where the format information goes, for a given mask. */
function placeFormat(g: Grid, mask: number): void {
  const size = g.length;
  const data = (EC_LEVEL_M << 3) | mask;
  let rem = data;
  for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537);
  const bits = (((data << 10) | rem) ^ 0x5412) >>> 0;

  for (let i = 0; i <= 5; i++) g[8][i] = ((bits >>> i) & 1) === 1;
  g[8][7] = ((bits >>> 6) & 1) === 1;
  g[8][8] = ((bits >>> 7) & 1) === 1;
  g[7][8] = ((bits >>> 8) & 1) === 1;
  for (let i = 9; i < 15; i++) g[14 - i][8] = ((bits >>> i) & 1) === 1;

  for (let i = 0; i < 8; i++) g[size - 1 - i][8] = ((bits >>> i) & 1) === 1;
  for (let i = 8; i < 15; i++) g[8][size - 15 + i] = ((bits >>> i) & 1) === 1;
  g[size - 8][8] = true;
}

/** The format bits for level M and a mask, exposed so a test can pin them. */
export function formatBits(mask: number): number {
  const data = (EC_LEVEL_M << 3) | mask;
  let rem = data;
  for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537);
  return (((data << 10) | rem) ^ 0x5412) >>> 0;
}

const MASKS: ((r: number, c: number) => boolean)[] = [
  (r, c) => (r + c) % 2 === 0,
  (r) => r % 2 === 0,
  (_r, c) => c % 3 === 0,
  (r, c) => (r + c) % 3 === 0,
  (r, c) => (Math.floor(r / 2) + Math.floor(c / 3)) % 2 === 0,
  (r, c) => ((r * c) % 2) + ((r * c) % 3) === 0,
  (r, c) => (((r * c) % 2) + ((r * c) % 3)) % 2 === 0,
  (r, c) => (((r + c) % 2) + ((r * c) % 3)) % 2 === 0,
];

/** Lay the codewords in: two columns at a time, upward then downward. */
function placeData(g: Grid, codewords: number[]): void {
  const size = g.length;
  let bit = 0;
  const next = (): boolean => {
    const i = bit >>> 3;
    const b = i < codewords.length ? (codewords[i] >>> (7 - (bit & 7))) & 1 : 0;
    bit++;
    return b === 1;
  };
  let upward = true;
  for (let right = size - 1; right >= 1; right -= 2) {
    // Column 6 is the vertical timing pattern and is not part of the path.
    if (right === 6) right = 5;
    for (let step = 0; step < size; step++) {
      const row = upward ? size - 1 - step : step;
      for (const col of [right, right - 1]) {
        if (g[row][col] === null) g[row][col] = next();
      }
    }
    upward = !upward;
  }
}

/** The standard's four penalties, lower being a code that reads more easily. */
function penalty(m: boolean[][]): number {
  const size = m.length;
  let score = 0;

  // Rule 1: runs of five or more of one colour, in rows and in columns.
  for (let i = 0; i < size; i++) {
    for (const line of [m[i], m.map((row) => row[i])]) {
      let run = 1;
      for (let j = 1; j < size; j++) {
        if (line[j] === line[j - 1]) {
          run++;
        } else {
          if (run >= 5) score += run - 2;
          run = 1;
        }
      }
      if (run >= 5) score += run - 2;
    }
  }

  // Rule 2: every 2x2 block of one colour.
  for (let r = 0; r < size - 1; r++) {
    for (let c = 0; c < size - 1; c++) {
      if (m[r][c] === m[r][c + 1] && m[r][c] === m[r + 1][c] && m[r][c] === m[r + 1][c + 1]) score += 3;
    }
  }

  // Rule 3: the finder-like 1:1:3:1:1 sequence with four light modules beside
  // it, which a reader could mistake for a finder pattern.
  const DARK_LIGHT = [true, false, true, true, true, false, true];
  const matches = (line: boolean[], at: number, pattern: boolean[]): boolean =>
    pattern.every((want, k) => line[at + k] === want);
  for (let i = 0; i < size; i++) {
    for (const line of [m[i], m.map((row) => row[i])]) {
      for (let j = 0; j + 7 <= size; j++) {
        if (!matches(line, j, DARK_LIGHT)) continue;
        const before = j - 4 < 0 || line.slice(Math.max(0, j - 4), j).every((v) => !v);
        const after = j + 11 > size || line.slice(j + 7, j + 11).every((v) => !v);
        if ((j - 4 < 0 || before) && (j + 11 > size || after) && (before || after)) score += 40;
      }
    }
  }

  // Rule 4: how far the proportion of dark modules strays from half.
  const dark = m.reduce((n, row) => n + row.filter(Boolean).length, 0);
  const percent = (dark * 100) / (size * size);
  score += Math.floor(Math.abs(percent - 50) / 5) * 10;

  return score;
}

export type Qr = { version: number; size: number; modules: boolean[][] };

/**
 * Encode `text` as a QR code at level M, choosing the smallest version that
 * fits and the mask that scores best.
 *
 * Throws when the text is longer than version 20 holds; the caller decides
 * what to say, because "the code could not be made" means different things on
 * different screens.
 */
export function encodeQr(text: string): Qr {
  const bytes = new TextEncoder().encode(text);
  const version = BLOCKS.findIndex((_, i) => byteCapacity(i + 1) >= bytes.length) + 1;
  if (version === 0) {
    throw new Error(`${bytes.length} bytes is more than a QR code of version ${MAX_VERSION} holds`);
  }
  const codewords = interleave(dataCodewords(bytes, version), version);
  const size = sizeOf(version);

  const base = blank(size);
  placeFunctions(base, version);
  // Which modules the mask may touch: everything data placement filled in.
  const free = base.map((row) => row.map((v) => v === null));
  placeData(base, codewords);

  let best: boolean[][] | null = null;
  let bestScore = Infinity;
  let bestMask = 0;
  for (let mask = 0; mask < 8; mask++) {
    const candidate = base.map((row, r) => row.map((v, c) => (free[r][c] && MASKS[mask](r, c) ? !v : !!v)));
    const withFormat = candidate.map((row) => row.slice());
    placeFormat(withFormat as Grid, mask);
    const score = penalty(withFormat as boolean[][]);
    if (score < bestScore) {
      bestScore = score;
      best = withFormat as boolean[][];
      bestMask = mask;
    }
  }
  void bestMask;
  return { version, size, modules: best! };
}

/**
 * The dark modules as one SVG path, in a viewBox of `size + 2 * quiet`.
 *
 * One path rather than a rectangle each: a version 12 code is over 2000
 * modules, and that many elements is slow to lay out and heavy in the DOM.
 */
export function qrPath(qr: Qr, quiet = 2): string {
  const parts: string[] = [];
  for (let r = 0; r < qr.size; r++) {
    for (let c = 0; c < qr.size; c++) {
      if (qr.modules[r][c]) parts.push(`M${c + quiet} ${r + quiet}h1v1h-1z`);
    }
  }
  return parts.join("");
}
