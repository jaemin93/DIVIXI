/** Which settings a window other than the PC's own keeps: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { isLayoutSetting, isTypeSetting, keepsSetting, LAYOUT_SETTINGS, TYPE_SETTINGS } from "./layoutSettings.ts";

test("from a phone the PC's layout is neither written nor read back", () => {
  for (const key of ["panel", "panel_width", "terminal", "terminal_height", "rail", "tracklist", "track", "track_headers", "ws_open", "tracks_filter"]) {
    assert.equal(keepsSetting(key, false), false, key);
  }
});

test("type and its size are each device's own", () => {
  for (const key of ["ui_font", "chat_font", "zoom"]) {
    assert.equal(isTypeSetting(key), true, key);
    assert.equal(keepsSetting(key, false), false, key);
    assert.equal(keepsSetting(key, true), true, key);
  }
});

test("on the PC itself every setting is kept as before", () => {
  for (const key of [...LAYOUT_SETTINGS, ...TYPE_SETTINGS]) assert.equal(keepsSetting(key, true), true, key);
});

test("theme, language and the work stay shared with the PC", () => {
  for (const key of ["theme", "language", "tags", "queued", "commands:claude_code", "sessions.idle_minutes", "knowledge.agent"]) {
    assert.equal(isLayoutSetting(key) || isTypeSetting(key), false, key);
    assert.equal(keepsSetting(key, false), true, key);
  }
});

test("the two lists do not overlap", () => {
  for (const key of TYPE_SETTINGS) assert.equal(isLayoutSetting(key), false, key);
});

test("only whole keys count, not ones that merely start alike", () => {
  assert.equal(isLayoutSetting("panelist"), false);
  assert.equal(isLayoutSetting("track_color"), false);
  assert.equal(isTypeSetting("chat_font_family"), false);
  assert.equal(isLayoutSetting(""), false);
});
