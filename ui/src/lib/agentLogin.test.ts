/** Telling a lost login from other failures, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { commandFromHere, wantsLogin } from "./agentLogin.ts";

test("the errors a lost login gives, as real runs recorded them", () => {
  assert.equal(wantsLogin('Internal error: Failed to authenticate: OAuth session expired and could not be refreshed ({"errorKind":"authentication_failed"})'), true);
  assert.equal(wantsLogin("Authentication required"), true);
});

test("other failures are shown as they are", () => {
  assert.equal(wantsLogin("Internal error: Failed to refresh OAuth token: another Claude Code process is refreshing it or exited mid-refresh."), false);
  assert.equal(wantsLogin('Internal error: Prompt is too long ({"errorKind":"invalid_request"})'), false);
  assert.equal(wantsLogin(""), false);
  assert.equal(wantsLogin(undefined), false);
});

test("the command runs here, or on the machine ssh reaches", () => {
  assert.equal(commandFromHere("/x/claude auth login", null), "/x/claude auth login");
  assert.equal(commandFromHere("/x/claude auth login", ""), "/x/claude auth login");
  assert.equal(commandFromHere("/x/claude auth login", "many"), "ssh -t many '/x/claude auth login'");
  assert.equal(commandFromHere("/it's/claude auth login", "many"), "ssh -t many '/it'\\''s/claude auth login'");
});
