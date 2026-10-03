/** The terminal palettes stay readable on their own backgrounds: `node --test`. */

import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { MIN_CONTRAST, ansiFor, contrast, type Ansi } from "./termTheme.ts";

/** `--bg` of a theme, read from the tokens so the two cannot drift apart. */
function bg(theme: "dk" | "lt"): string {
  const css = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");
  const block = css.match(new RegExp(`\\.${theme} \\{([^}]*)\\}`))?.[1] ?? "";
  const hex = block.match(/--bg:\s*(#[0-9a-f]{6})/i)?.[1];
  assert.ok(hex, `--bg for .${theme}`);
  return hex;
}

test("contrast matches the WCAG reference points", () => {
  assert.equal(contrast("#000000", "#ffffff"), 21);
  assert.equal(contrast("#777777", "#777777"), 1);
  assert.ok(Math.abs(contrast("#767676", "#ffffff") - 4.54) < 0.01);
});

test("every light colour reads on the light background", () => {
  const back = bg("lt");
  for (const [name, hex] of Object.entries(ansiFor("lt"))) {
    const ratio = contrast(hex, back);
    assert.ok(ratio >= MIN_CONTRAST, `${name} ${hex} on ${back}: ${ratio.toFixed(2)}`);
  }
});

test("every dark colour but black reads on the dark background", () => {
  const back = bg("dk");
  for (const [name, hex] of Object.entries(ansiFor("dk"))) {
    if (name === "black") continue; // black on black is what a program asked for
    const ratio = contrast(hex, back);
    assert.ok(ratio >= MIN_CONTRAST, `${name} ${hex} on ${back}: ${ratio.toFixed(2)}`);
  }
});

test("the reported colours: yellow and grey", () => {
  const keys: (keyof Ansi)[] = ["yellow", "brightYellow", "brightBlack"];
  for (const theme of ["dk", "lt"] as const) {
    for (const k of keys) assert.ok(contrast(ansiFor(theme)[k], bg(theme)) >= MIN_CONTRAST, `${theme} ${k}`);
  }
});
