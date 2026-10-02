/** The name the pairing page sends for itself: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { pairingName } from "./deviceName.ts";

const IPAD_SAFARI = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15";
const IPAD_EDGE = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 EdgiOS/131.0 Safari/605.1.15";
const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Mobile/15E148 Safari/604.1";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36 Edg/120.0";

test("an iPad that says Macintosh is named an iPad, whatever its browser", () => {
  assert.equal(pairingName(IPAD_SAFARI, 5), "iPad");
  assert.equal(pairingName(IPAD_EDGE, 5), "iPad");
});

test("a Mac, which has no touch screen, is left to the server", () => {
  assert.equal(pairingName(IPAD_SAFARI, 0), undefined);
  assert.equal(pairingName(IPAD_SAFARI, 1), undefined);
});

test("other devices are left to the server, and no browser is ever named", () => {
  assert.equal(pairingName(IPHONE, 5), undefined);
  assert.equal(pairingName(WINDOWS, 10), undefined);
  assert.equal(pairingName("", 0), undefined);
});
