/** The QR encoder, against the standard's own numbers: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { MAX_VERSION, byteCapacity, dataCapacity, encodeQr, errorCorrection, formatBits, qrPath, sizeOf, totalCodewords } from "./qr.ts";

/**
 * ISO/IEC 18004 table 9: every codeword a version holds, data and error
 * correction together. The block table in qr.ts is five numbers per version
 * and a slip in any of them is silent, so each row is cross-checked against
 * the one total the standard states separately.
 */
const TOTAL_CODEWORDS = [26, 44, 70, 100, 134, 172, 196, 242, 292, 346, 404, 466, 532, 581, 655, 733, 815, 901, 991, 1085];

test("every version's blocks add up to the standard's total codewords", () => {
  assert.equal(MAX_VERSION, TOTAL_CODEWORDS.length);
  for (let v = 1; v <= MAX_VERSION; v++) {
    assert.equal(totalCodewords(v), TOTAL_CODEWORDS[v - 1], `version ${v}`);
  }
});

test("Reed-Solomon matches the worked example", () => {
  // The standard's own "HELLO WORLD" example at version 1-Q: these data
  // codewords produce exactly these ten error-correction codewords. It
  // exercises the field arithmetic, the generator polynomial and the
  // remainder together, which is the part that fails silently.
  const data = [32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17];
  const want = [196, 35, 39, 119, 235, 215, 231, 226, 93, 23];
  assert.deepEqual(errorCorrection(data, 10), want);
});

test("Reed-Solomon of nothing is nothing", () => {
  assert.deepEqual(errorCorrection(new Uint8Array(16), 10), new Array(10).fill(0));
});

test("format information matches the published bits for level M", () => {
  // Level M with mask 0 is 101010000010010, the value every reference prints.
  assert.equal(formatBits(0), 0b101010000010010);
  // All eight are 15 bits, and no two are the same -- the BCH exists so a
  // reader can tell them apart under damage.
  const seen = new Set<number>();
  for (let mask = 0; mask < 8; mask++) {
    const bits = formatBits(mask);
    assert.ok(bits <= 0x7fff, `mask ${mask} is 15 bits`);
    seen.add(bits);
  }
  assert.equal(seen.size, 8);
});

test("a version's size and capacity follow the standard", () => {
  assert.equal(sizeOf(1), 21);
  assert.equal(sizeOf(7), 45);
  assert.equal(sizeOf(20), 97);
  // Level M, byte mode, from the standard's capacity table.
  assert.equal(byteCapacity(1), 14);
  assert.equal(byteCapacity(10), 213);
  assert.equal(dataCapacity(1), 16);
});

test("the smallest version that fits is the one chosen", () => {
  assert.equal(encodeQr("x".repeat(14)).version, 1);
  assert.equal(encodeQr("x".repeat(15)).version, 2);
  assert.equal(encodeQr("x".repeat(213)).version, 10);
  assert.equal(encodeQr("x".repeat(214)).version, 11);
});

test("a payload too long is refused rather than truncated", () => {
  assert.throws(() => encodeQr("x".repeat(byteCapacity(MAX_VERSION) + 1)), /more than a QR code/);
});

test("the text is measured in bytes, not characters", () => {
  // Fourteen characters fit version 1; fourteen Korean syllables do not,
  // because each is three bytes of UTF-8.
  assert.equal(encodeQr("x".repeat(14)).version, 1);
  assert.ok(encodeQr("가".repeat(14)).version > 1);
});

/** Every module of a finder pattern, as the standard draws it. */
const FINDER = [
  [1, 1, 1, 1, 1, 1, 1],
  [1, 0, 0, 0, 0, 0, 1],
  [1, 0, 1, 1, 1, 0, 1],
  [1, 0, 1, 1, 1, 0, 1],
  [1, 0, 1, 1, 1, 0, 1],
  [1, 0, 0, 0, 0, 0, 1],
  [1, 1, 1, 1, 1, 1, 1],
].map((row) => row.map((v) => v === 1));

test("the fixed patterns are where a reader looks for them", () => {
  const qr = encodeQr("https://laptop-mc28nnvi.tailb45a71.ts.net/auth/pair?token=abc.def");
  const m = qr.modules;
  const at = (top: number, left: number) =>
    FINDER.every((row, r) => row.every((want, c) => m[top + r][left + c] === want));
  assert.ok(at(0, 0), "top left");
  assert.ok(at(0, qr.size - 7), "top right");
  assert.ok(at(qr.size - 7, 0), "bottom left");

  // The timing patterns alternate, starting dark at module 8.
  for (let i = 8; i < qr.size - 8; i++) {
    assert.equal(m[6][i], i % 2 === 0, `row timing at ${i}`);
    assert.equal(m[i][6], i % 2 === 0, `column timing at ${i}`);
  }
  // The module that is dark in every code.
  assert.equal(m[qr.size - 8][8], true);
});

test("the separators around the finders stay light", () => {
  const qr = encodeQr("divixi");
  const m = qr.modules;
  for (let i = 0; i < 8; i++) {
    assert.equal(m[7][i], false, `below top-left at ${i}`);
    assert.equal(m[i][7], false, `right of top-left at ${i}`);
  }
});

test("the grid is square, fully decided, and roughly half dark", () => {
  const qr = encodeQr("https://laptop-mc28nnvi.tailb45a71.ts.net/auth/pair?token=" + "a".repeat(180));
  assert.equal(qr.modules.length, qr.size);
  for (const row of qr.modules) {
    assert.equal(row.length, qr.size);
    for (const v of row) assert.equal(typeof v, "boolean", "no module left unplaced");
  }
  // Mask selection exists to keep this near half; far from it would mean the
  // masks are not being applied or scored.
  const dark = qr.modules.reduce((n, row) => n + row.filter(Boolean).length, 0);
  const share = dark / (qr.size * qr.size);
  assert.ok(share > 0.35 && share < 0.65, `dark share ${share}`);
});

test("encoding is deterministic", () => {
  const text = "https://laptop-mc28nnvi.tailb45a71.ts.net/auth/pair?token=abc.def";
  assert.deepEqual(encodeQr(text).modules, encodeQr(text).modules);
});

test("a one-character change changes the code", () => {
  const a = encodeQr("https://example.ts.net/auth/pair?token=aaa.bbb");
  const b = encodeQr("https://example.ts.net/auth/pair?token=aaa.bbc");
  assert.notDeepEqual(a.modules, b.modules);
});

test("the path draws one square per dark module, inside the quiet zone", () => {
  const qr = encodeQr("divixi");
  const path = qrPath(qr, 2);
  const dark = qr.modules.reduce((n, row) => n + row.filter(Boolean).length, 0);
  assert.equal(path.split("M").length - 1, dark);
  // The top-left finder's first module sits at the quiet-zone offset.
  assert.ok(path.startsWith("M2 2h1v1h-1z"), path.slice(0, 20));
});

test("a pairing URL the longest tailnet name could produce still fits", () => {
  // 253 is the longest a DNS name can be, and the token is about 190
  // characters. If this ever throws, the version table has to grow.
  const url = `https://${"a".repeat(60)}.${"b".repeat(60)}.ts.net/auth/pair?token=${"t".repeat(200)}`;
  const qr = encodeQr(url);
  assert.ok(qr.version <= MAX_VERSION);
});
