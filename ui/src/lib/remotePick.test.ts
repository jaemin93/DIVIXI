/** The remote picker's paths and selection: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { canPick, extOf, folderOf, joinPath, sizeLabel, toggle } from "./remotePick.ts";

test("a name joins a folder with the separator it already uses", () => {
  assert.equal(joinPath("C:\\Users\\me", "docs"), "C:\\Users\\me\\docs");
  assert.equal(joinPath("C:\\", "docs"), "C:\\docs");
  assert.equal(joinPath("/home/me", "docs"), "/home/me/docs");
  assert.equal(joinPath("/", "etc"), "/etc");
});

test("the folder of a picked file, roots kept whole", () => {
  assert.equal(folderOf("/home/me/notes.md"), "/home/me");
  assert.equal(folderOf("/notes.md"), "/");
  assert.equal(folderOf("C:\\Users\\me\\notes.md"), "C:\\Users\\me");
  assert.equal(folderOf("C:\\notes.md"), "C:\\");
  assert.equal(folderOf("notes.md"), "");
});

test("extensions are lowercased; a dotfile has none", () => {
  assert.equal(extOf("Plan.MD"), "md");
  assert.equal(extOf("archive.tar.gz"), "gz");
  assert.equal(extOf(".env"), "");
  assert.equal(extOf("README"), "");
});

test("sizes read as the files panel shows them", () => {
  assert.equal(sizeLabel(0), "0 B");
  assert.equal(sizeLabel(2048), "2.0 KB");
  assert.equal(sizeLabel(3 * 1048576), "3.0 MB");
});

test("only files of a kind the library reads may be picked", () => {
  const formats = ["md", "pdf"];
  assert.equal(canPick({ name: "a.md", dir: false, size: 1 }, formats), true);
  assert.equal(canPick({ name: "A.PDF", dir: false, size: 1 }, formats), true);
  assert.equal(canPick({ name: "a.exe", dir: false, size: 1 }, formats), false);
  assert.equal(canPick({ name: "docs.md", dir: true, size: 0 }, formats), false, "a folder is entered, not picked");
  assert.equal(canPick({ name: "a.exe", dir: false, size: 1 }, []), true, "no list yet: the instance decides");
});

test("picking twice takes it back", () => {
  let s = toggle([], "/a.md");
  s = toggle(s, "/b.md");
  assert.deepEqual(s, ["/a.md", "/b.md"]);
  assert.deepEqual(toggle(s, "/a.md"), ["/b.md"]);
});
