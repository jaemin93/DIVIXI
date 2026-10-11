/** What a screen remembers where the PC's settings cannot: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { clampZoom, FIELD_MIN_PX, fieldFontPx, foldsOnPick, foldValue, instanceZoomKey, LIST_FOLDS, listAfterRail, parseZoom, settingOpen, WEB_RAIL_KEY, WEB_TRACKLIST_KEY, webPanelOpen, ZOOM_MAX, ZOOM_MIN } from "./uiMemory.ts";
import { isLayoutSetting, keepsSetting } from "./layoutSettings.ts";

test("a zoom asked for lands on a step within bounds", () => {
  assert.equal(clampZoom(110), 110);
  assert.equal(clampZoom(104), 100);
  assert.equal(clampZoom(106), 110);
  assert.equal(clampZoom(10), ZOOM_MIN);
  assert.equal(clampZoom(900), ZOOM_MAX);
  assert.equal(clampZoom(Number.NaN), 100);
  assert.equal(clampZoom(Number.POSITIVE_INFINITY), 100);
});

test("a stored zoom comes back as it was kept", () => {
  assert.equal(parseZoom("110"), 110);
  assert.equal(parseZoom(" 90 "), 90);
  assert.equal(parseZoom(String(ZOOM_MIN)), ZOOM_MIN);
  assert.equal(parseZoom(String(ZOOM_MAX)), ZOOM_MAX);
  assert.equal(parseZoom("115"), 120);
});

test("a stored zoom that is broken is not trusted", () => {
  for (const raw of [null, undefined, "", "abc", "110%", "-110", "1e3", "NaN", "Infinity", "5000", "49", "201", "0x6e", "110 120"]) {
    assert.equal(parseZoom(raw), null, String(raw));
  }
});

test("each remote instance keeps a zoom of its own", () => {
  assert.equal(instanceZoomKey("office-server"), "instance:office-server:zoom");
  assert.notEqual(instanceZoomKey("office-server"), instanceZoomKey("remote-1"));
  // Not a key the PC's own window uses for its zoom.
  assert.notEqual(instanceZoomKey("office-server"), "zoom");
});

test("a phone's panels start closed until they are opened", () => {
  assert.equal(webPanelOpen(null, true), false);
  assert.equal(webPanelOpen(undefined, true), false);
  assert.equal(webPanelOpen(null, false), true);
});

test("a panel comes back as it was left on this device", () => {
  assert.equal(webPanelOpen("open", true), true);
  assert.equal(webPanelOpen("closed", true), false);
  assert.equal(webPanelOpen("closed", false), false);
  assert.equal(webPanelOpen("open", false), true);
});

test("a stored panel state that is neither is the default", () => {
  for (const raw of ["", "true", "OPEN", "collapsed"]) {
    assert.equal(webPanelOpen(raw, true), false, raw);
    assert.equal(webPanelOpen(raw, false), true, raw);
  }
});

test("a browser's panels are kept apart from the desktop window's settings", () => {
  assert.notEqual(WEB_TRACKLIST_KEY, "tracklist");
  assert.notEqual(WEB_RAIL_KEY, "rail");
  assert.notEqual(WEB_TRACKLIST_KEY, WEB_RAIL_KEY);
});

test("only a narrow browser folds the list when a track is picked", () => {
  assert.equal(foldsOnPick(true, true), true);
  assert.equal(foldsOnPick(true, false), false);
  // The desktop window, however narrow, keeps its list as before.
  assert.equal(foldsOnPick(false, true), false);
  assert.equal(foldsOnPick(false, false), false);
});

test("a phone's field is 16px on screen at 100% and zoomed in", () => {
  assert.equal(FIELD_MIN_PX, 16);
  assert.equal(fieldFontPx(100), 16);
  assert.equal(fieldFontPx(110), 16);
  assert.equal(fieldFontPx(ZOOM_MAX), 16);
  // Not a number: the page is at 100%, and so is the field.
  assert.equal(fieldFontPx(Number.NaN), 16);
});

test("a phone zoomed out grows its fields back to 16px on screen", () => {
  assert.equal(fieldFontPx(50), 32);
  assert.equal(fieldFontPx(80), 20);
  assert.equal(fieldFontPx(90), 17.78);
  assert.equal(fieldFontPx(10), 32); // clamped to ZOOM_MIN first
  for (let z = ZOOM_MIN; z <= ZOOM_MAX; z += 10) {
    assert.ok((fieldFontPx(z) * z) / 100 >= FIELD_MIN_PX, String(z));
  }
});

test("a list column folded comes back folded, and open comes back open", () => {
  for (const open of [true, false]) {
    // The desktop window, through the PC's setting.
    assert.equal(settingOpen(foldValue(open)), open);
    // A browser, through its own storage, on a phone and elsewhere alike.
    assert.equal(webPanelOpen(foldValue(open), true), open);
    assert.equal(webPanelOpen(foldValue(open), false), open);
  }
});

test("a list column with nothing kept starts open in the desktop window", () => {
  assert.equal(settingOpen(null), true);
  assert.equal(settingOpen(undefined), true);
  assert.equal(settingOpen(""), true);
  assert.equal(settingOpen("collapsed"), true);
  assert.equal(settingOpen("closed"), false);
});

test("every list column folds, the routines column too", () => {
  assert.deepEqual(Object.keys(LIST_FOLDS).sort(), ["designs", "libraries", "routines", "tracks"]);
  assert.equal(LIST_FOLDS.routines.setting, "routinelist");
  assert.equal(LIST_FOLDS.routines.web, "divixi.routinelist");
});

test("no two list columns keep their fold under the same key", () => {
  const lists = Object.values(LIST_FOLDS);
  const keys = [...lists.map((l) => l.setting), ...lists.map((l) => l.web), WEB_RAIL_KEY, "rail"];
  assert.equal(new Set(keys).size, keys.length);
  // Not the width kept beside it either.
  assert.ok(lists.every((l) => !l.setting.endsWith("_width")));
});

test("a list column's fold is the window's layout, not shared with a phone", () => {
  for (const l of Object.values(LIST_FOLDS)) {
    assert.ok(isLayoutSetting(l.setting), l.setting);
    assert.equal(keepsSetting(l.setting, false), false);
    assert.equal(keepsSetting(l.setting, true), true);
    // A browser's key is its own, never the PC's setting.
    assert.ok(l.web.startsWith("divixi."), l.web);
    assert.notEqual(l.web, l.setting);
  }
});

test("coming to a page from the rail leaves its list as it was last left", () => {
  assert.equal(listAfterRail(false, false), false);
  assert.equal(listAfterRail(true, false), true);
});

test("pressing the rail item of the page shown folds its list, and back", () => {
  assert.equal(listAfterRail(true, true), false);
  assert.equal(listAfterRail(false, true), true);
});

test("folded, restarted, come back to: still folded, and a second press opens it", () => {
  // Desktop: folded with the head's button, kept as the PC's setting.
  let kept = foldValue(false);
  // Restart: read back, then the rail from the tracks page, then again on the page.
  let open = settingOpen(kept);
  open = listAfterRail(open, false);
  assert.equal(open, false);
  open = listAfterRail(open, true);
  assert.equal(open, true);
  kept = foldValue(open);
  assert.equal(settingOpen(kept), true);
});

test("a phone's lists still start folded, are kept on the phone, and fold on a pick (#66)", () => {
  // Nothing kept on this phone: folded, and coming to the page does not open it.
  let open = webPanelOpen(null, true);
  assert.equal(open, false);
  open = listAfterRail(open, false);
  assert.equal(open, false);
  // A second press opens it, and the phone keeps that for its next start.
  open = listAfterRail(open, true);
  let stored = foldValue(open);
  assert.equal(webPanelOpen(stored, true), true);
  // Picking a row folds it on a narrow phone, and that is kept too.
  assert.equal(foldsOnPick(true, true), true);
  stored = foldValue(false);
  assert.equal(listAfterRail(webPanelOpen(stored, true), false), false);
});
