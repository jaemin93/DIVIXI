import { test } from "node:test";
import assert from "node:assert/strict";

import { DESIGN_EXT, IMPORT_MAX_MB, tooLarge, WEB_IMPORT_MAX_MB } from "./designFile.ts";

const MB = 1024 * 1024;

test("a design file may be as large as the PC reads, less from a browser", () => {
  assert.equal(tooLarge(IMPORT_MAX_MB * MB, false), null);
  assert.equal(tooLarge(IMPORT_MAX_MB * MB + 1, false), IMPORT_MAX_MB);
  assert.equal(tooLarge(WEB_IMPORT_MAX_MB * MB, true), null);
  assert.equal(tooLarge(WEB_IMPORT_MAX_MB * MB + 1, true), WEB_IMPORT_MAX_MB, "the server's request limit, not the PC's");
  assert.ok(WEB_IMPORT_MAX_MB < 72, "under the PC server's 72 MB a request");
});

test("the extension is the core's", () => {
  assert.equal(DESIGN_EXT, ".divixi-design");
});
