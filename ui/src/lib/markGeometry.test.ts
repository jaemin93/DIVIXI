import test from "node:test";
import assert from "node:assert/strict";
import { staves } from "./markGeometry.ts";

const SIZES = [11, 12, 14, 16, 20, 24, 28, 40, 56, 168];
const RATIOS = [1, 1.25, 1.5, 1.75, 2, 2.5, 3];

test("four staves at every size and pixel ratio, each a whole pixel with a gap between", () => {
  for (const size of SIZES) {
    for (const dpr of RATIOS) {
      const st = staves(size, dpr);
      const at = `${size}px @${dpr}x`;
      assert.equal(st.rows.length, 4, at);
      assert.ok(Number.isInteger(st.thick) && st.thick >= 1, at);
      for (const r of st.rows) assert.ok(Number.isInteger(r), at);
      for (let i = 1; i < 4; i++) assert.ok(st.rows[i] - st.rows[i - 1] >= st.thick + 1, `${at}: a gap between staves`);
      assert.ok(st.rows[0] >= 0 && st.rows[3] + st.thick <= Math.floor(st.box), `${at}: inside the box`);
      assert.ok(st.x1 > st.x0, at);
    }
  }
});

test("where there is room, the staves are where the app icon draws them", () => {
  // 64 device pixels: one unit is one pixel, staves 2 thick at 22/30/38/46.
  assert.deepEqual(staves(64, 1), { box: 64, rows: [21, 29, 37, 45], thick: 2, x0: 8, x1: 56 });
  assert.deepEqual(staves(16, 2).rows, staves(32, 1).rows);
});

test("the staves stay centred on the X", () => {
  for (const size of SIZES) {
    const st = staves(size, 1);
    const mid = (st.rows[0] + st.rows[3] + st.thick) / 2;
    assert.ok(Math.abs(mid - (34 * st.box) / 64) <= 1, `${size}px: centre ${mid}`);
  }
});
