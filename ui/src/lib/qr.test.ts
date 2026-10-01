/**
 * The QR integration: `node --test`.
 *
 * Not a test of the standard — `qrcode-generator` is the reference
 * implementation and does not need checking here. This pins the two things
 * that are ours and could drift: which encoding options we ask for (byte
 * mode, level M, automatic version) and the path we draw from the grid.
 *
 * The fixture is the library's own output for a realistic pairing URL. The
 * hand-written encoder this replaced agreed with the standard on version,
 * size, Reed-Solomon and format bits, and still differed from a correct
 * symbol in 372 of 1681 modules; a phone camera read the picture as text.
 * Nothing short of comparing the whole grid would have said so.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { encodeQr, qrPath } from "./qr.ts";

const PAYLOAD =
  "https://laptop-a1b2c3d4.tail123456.ts.net/?token=eyJrIjoicGFpciIsImV4cCI6MTc5MDAwMDAwMCwibiI6IjAxMjM0NTY3ODlhYmNkZWYwMTIzNDU2Nzg5YWJjZGVmIiwiZyI6MCwicyI6ImNvbnZlcnNhdGlvbiIsImwiOjYwNDgwMH0.bXlzaWduYXR1cmVteXNpZ25hdHVyZW15c2lnbmF0dXJlMQ";

/**
 * The module grid `qrcode-generator` produces for PAYLOAD at level M with
 * the version left automatic, taken from the library itself and pinned here.
 *
 * This is the test that would have caught the hand-written encoder: it
 * agreed with the standard on version, size, Reed-Solomon and format bits,
 * and still differed from a correct symbol in 372 of 1681 modules. Nothing
 * short of the whole grid would have said so.
 */
const MODULES = [
  "#######...#.......####..#...##.#...#.#.######..#...##.#######",
  "#.....#..##.##.#.#..###..#.##....#.###.##.#..#####.##.#.....#",
  "#.###.#.#....#....#.........####..#.##..##...###..###.#.###.#",
  "#.###.#.##.#...###...#####.##....####..#..##.######.#.#.###.#",
  "#.###.#.#####..#..##....#.#.#####.#..#..###.#..#####..#.###.#",
  "#.....#.###.#....###...##.###...##...#.....##.....#...#.....#",
  "#######.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#.#######",
  "........#..##..##.###..###..#...#...####.##..#..#............",
  "#.#####..##..#.##..#.#####..#########.....##...###.##.#####..",
  ".#.##...##.#..#...#.#####.#....#.#..#.....####.#.#...##.###.#",
  "..#...##......#.#.##.##..#..####.######..#....#.####....#..##",
  "#...#....###..#..#..#..##.##.##.##.#.####...###..#..##.###..#",
  ".#.#.##..##.#..#.###..#..#..........##...###...##.####....#.#",
  ".#...#.#.....###.....###...######..###....###..###...####.##.",
  "###..####..#.#..#.#..#####.#.#.#..###..###...###.##...#....##",
  ".#...#...###..##....###.#....###....##.####.#..########....##",
  "##.#..#..##....#####...###.##...#.#.##....##..####.#...#...##",
  ".##.#...#.#....##..####..##.....#..###.#.##.#..###...##..##..",
  "#..#..#..#..###.###..#......#.##....##.#.##..#......#...##.##",
  "...###.#...#.##...##.#...#...####.##..###..##.#.#....#.##..#.",
  "##.##.#.#.#.##.#...##...#...#..#...###...###.#.######..#..#..",
  "..##....#..####...#.#.#..#..##..#..#.#..####....##...##.###..",
  "...#.#####.#.##..####....##....#.#.####.#...#.###.##.#.##..##",
  "#...#..###.#.###..##.#..####.#..##.##.####.....##.#.#......#.",
  "##..###..###..#.#....#####....###..##......#..###...#..#.#..#",
  "#.##.#.#..#..#..#....#..#.##.##..#...#.####.##.#.#.#..###.#.#",
  "....#.#.##...#.####.#####.#..####......##...#.###.###....####",
  "#.#..#.##.##...##.#..#..#.#..##.#.#.#...###..#.#...#.#..#..##",
  ".##.#####...###....#.###.#..##########.#..##...##..######....",
  ".#.##...#.#....#.####.#.#..##...#..##...#.###..###..#...#..#.",
  "#####.#.#.##...#.###.#...####.#.#.##.###.....##..##.#.#.###.#",
  "#####...##.#....##..##..##.##...#..#.#.######.#....##...#..##",
  "#########..##...#..###...#.######.#.#...........#..#######.#.",
  ".#.....###.#.#..#.####.##..###.##...##...####...##...#.#.....",
  "#.....#..############.####.##.#.##.##..#..#.....#.#.####.####",
  "..#..#.#.##.#....#.#..#..#.#...#...##.#.#####.##.####...#..##",
  ".#.####.#.##....##..##...#..###.######.#.##....##.##.####.##.",
  "....#...#....####.###.####.....##..###.##.###....#.#..##.....",
  "#.##.##..##..#..#.#.#.#######...####.#...#.....##.#..##..#.#.",
  "##.##..#.#.####.##..#.##..##.###.##..#.#.#.......####.......#",
  ".#..#.#####.#####..###..###.####.#.####..###.####.###.##..#.#",
  ".#.###..#.#..#....####.#.....##.##..##...###....##.....#.##..",
  ".#.#####.##..##.##.##.#...####.##.###..###..###....#.###.####",
  "#..###....#.#.###.##.#.#.##.#.##..#...#.#.####.###..#..#...##",
  "..##..#...##.#....##.#####...########.#....#...##..##.#...#.#",
  "#...##..#.#..###.#.######.####....#..#.#####.#...#..#.##...#.",
  ".##.#.##.#.#....##.###..#.....#.##....#.#...#.#.....####.#.##",
  ".##..#...#..#...######...#..###.....#..###..##..##...####..#.",
  "#...#.#.....###.##..###..#....####.##.#....#.####..#.#.##...#",
  "#####..##..#.....#....#####.###....#...#.##..#.###...#...##.#",
  "..#######..###.###.#.###...#.###..#####..#.#..#...#.#.##...##",
  "###.#....##...#........####...#...##...###..#..#...##...#....",
  "####..#.#....###.....#..##..#####...##...###.##############..",
  "........####..#.#...#.#####.#...#..#.#.#.##.##.#.#.##...#..#.",
  "#######...#.#.###.#.##..#.#.#.#.##..###..##...##..#.#.#.##..#",
  "#.....#.###.#.##.##.##...#..#...#.#.##.#..#.#..##.#.#...#....",
  "#.###.#.#...###.##..#....#.######..####....#.#.####.########.",
  "#.###.#.#.#..####.#..#####.##.##....##...###....##.##.......#",
  "#.###.#.##.#..#####.##.##..##....#.#....#.##.#...#...###.#..#",
  "#.....#.....###.#.########.#....##..#######.##...###.##.#...#",
  "#######.##...#...#####.##....##..#.##.#..#.#.####.#.#.....###",
];

test("a pairing URL encodes to exactly the grid the library produces", () => {
  const qr = encodeQr(PAYLOAD);
  assert.equal(qr.size, MODULES.length);
  const drawn = qr.modules.map((row) => row.map((v) => (v ? "#" : ".")).join(""));
  assert.deepEqual(drawn, MODULES);
});

test("a realistic pairing URL is a version the quiet zone still fits on a card", () => {
  // 61 modules is version 11. It is recorded because the rendered size has to
  // follow it: at a fixed 148px this symbol is 2.3 screen pixels per module,
  // which no camera reads. PhoneQr sizes from the module count for that reason.
  assert.equal(encodeQr(PAYLOAD).size, 61);
});

test("the payload must be ASCII, and says so instead of corrupting", () => {
  // The library's byte conversion is `charCodeAt(i) & 0xff`, so anything above
  // U+00FF would encode to a different byte without complaint.
  assert.throws(() => encodeQr("https://example.ts.net/?token=가"), /must be ASCII/);
  assert.doesNotThrow(() => encodeQr("https://example.ts.net/?token=abc.def"));
});

test("encoding is deterministic and follows the payload", () => {
  assert.deepEqual(encodeQr(PAYLOAD).modules, encodeQr(PAYLOAD).modules);
  const other = encodeQr(PAYLOAD.slice(0, -1) + "2");
  assert.notDeepEqual(other.modules, encodeQr(PAYLOAD).modules);
});

test("the path draws one square per dark module, inside the quiet zone", () => {
  const qr = encodeQr(PAYLOAD);
  const path = qrPath(qr, 2);
  const dark = qr.modules.reduce((n, row) => n + row.filter(Boolean).length, 0);
  assert.equal(path.split("M").length - 1, dark);
  // The top-left finder's first module sits at the quiet-zone offset.
  assert.ok(path.startsWith("M2 2h1v1h-1z"), path.slice(0, 20));
});
