import test from "node:test";
import assert from "node:assert/strict";
import { LANGUAGES, languageForFile, resolveLanguage } from "./languages.ts";
import { LOADERS, colour, hasGrammar, highlight, loadLanguage, onGrammar } from "./highlight.ts";

test("fence tags resolve to grammars, whatever the spelling", () => {
  for (const tag of ["cpp", "c++", "C++", "cc", "cxx", "hpp", "hh", "hxx", "h"]) assert.equal(resolveLanguage(tag), "cpp", tag);
  assert.equal(resolveLanguage("c"), "c");
  assert.equal(resolveLanguage("c#"), "csharp");
  assert.equal(resolveLanguage("cs"), "csharp");
  assert.equal(resolveLanguage("objective-c"), "objectivec");
  assert.equal(resolveLanguage("golang"), "go");
  assert.equal(resolveLanguage("kt"), "kotlin");
  for (const tag of ["js", "jsx", "ts", "tsx", "mjs", "cjs", "javascript", "typescript"]) assert.equal(resolveLanguage(tag), "typescript", tag);
  for (const tag of ["sh", "bash", "zsh", "shell"]) assert.equal(resolveLanguage(tag), "bash", tag);
  for (const tag of ["ps1", "powershell", "pwsh"]) assert.equal(resolveLanguage(tag), "powershell", tag);
  assert.equal(resolveLanguage("console"), "shell");
  assert.equal(resolveLanguage("toml"), "ini");
  assert.equal(resolveLanguage("patch"), "diff");
  assert.equal(resolveLanguage("proto"), "protobuf");
  assert.equal(resolveLanguage("gql"), "graphql");
  assert.equal(resolveLanguage("html"), "xml");
  assert.equal(resolveLanguage("py"), "python");
  assert.equal(resolveLanguage("python"), "python");
});

test("a fence's extra words, braces and a leading dot do not matter", () => {
  assert.equal(resolveLanguage("cpp title=main.cpp"), "cpp");
  assert.equal(resolveLanguage("{.python}"), "python");
  assert.equal(resolveLanguage(" Rust,ignore"), "rust");
});

test("unknown or empty fence tags are plain", () => {
  for (const tag of ["", "   ", null, undefined, "text", "plaintext", "zig", "brainfuck", "c--"]) assert.equal(resolveLanguage(tag), null, String(tag));
});

test("file names map by extension, case-insensitively, from any path", () => {
  const cases: Record<string, string> = {
    "main.c": "c",
    "main.cpp": "cpp",
    "src/engine/Vector.HPP": "cpp",
    "a.cc": "cpp",
    "a.cxx": "cpp",
    "a.h": "cpp",
    "a.hh": "cpp",
    "a.hxx": "cpp",
    "Program.cs": "csharp",
    "Main.java": "java",
    "build.gradle.kts": "kotlin",
    "main.go": "go",
    "lib.rs": "rust",
    "app.jsx": "typescript",
    "app.tsx": "typescript",
    "index.mjs": "typescript",
    "index.cjs": "typescript",
    "App.swift": "swift",
    "View.m": "objectivec",
    "View.mm": "objectivec",
    "app.rb": "ruby",
    "index.php": "php",
    "Main.scala": "scala",
    "run.sh": "bash",
    "run.zsh": "bash",
    "C:\tools\setup.ps1": "powershell",
    "q.sql": "sql",
    "index.html": "xml",
    "site.css": "css",
    "site.scss": "scss",
    "data.json": "json",
    "ci.yml": "yaml",
    "Cargo.toml": "ini",
    "pom.xml": "xml",
    "README.md": "markdown",
    "init.lua": "lua",
    "stats.R": "r",
    "main.dart": "dart",
    "Main.hs": "haskell",
    "app.ex": "elixir",
    "fix.patch": "diff",
    "setup.cfg": "ini",
    "api.proto": "protobuf",
    "schema.graphql": "graphql",
    "train.py": "python",
  };
  for (const [name, id] of Object.entries(cases)) assert.equal(languageForFile(name), id, name);
});

test("whole file names say the language when the extension does not", () => {
  assert.equal(languageForFile("Dockerfile"), "dockerfile");
  assert.equal(languageForFile("docker/Dockerfile.dev"), "dockerfile");
  assert.equal(languageForFile("Makefile"), "makefile");
  assert.equal(languageForFile("GNUmakefile"), "makefile");
  assert.equal(languageForFile("CMakeLists.txt"), "cmake");
  assert.equal(languageForFile("Gemfile"), "ruby");
  assert.equal(languageForFile("dotfiles/.bashrc"), "bash");
});

test("files with no known language are plain", () => {
  for (const name of ["notes.txt", "LICENSE", "a.zig", "photo.png", "", null, undefined, "dir/"]) assert.equal(languageForFile(name), null, String(name));
});

test("every grammar in the table is bundled or has a loader, and every loader is in the table", () => {
  for (const id of Object.keys(LANGUAGES)) assert.ok(hasGrammar(id) || LOADERS[id], id);
  for (const id of Object.keys(LOADERS)) assert.ok(id in LANGUAGES, id);
});

test("a lazy grammar draws plain first, then coloured once it arrives", async () => {
  const src = "void main() { print('hi'); }";
  let calls = 0;
  const off = onGrammar(() => calls++);
  assert.equal(hasGrammar("dart"), false);
  assert.equal(colour(src, "dart"), null);
  // Asking sent for the grammar; until it is in, the text is drawn as is.
  assert.equal(highlight(src, "dart"), src);
  assert.equal(await loadLanguage("dart"), true);
  off();
  assert.equal(calls, 1);
  assert.match(highlight(src, "dart"), /<span class="hljs-keyword">void<\/span>/);
});

test("C++ colours under every spelling once loaded", async () => {
  assert.equal(await loadLanguage("c++"), true);
  const src = "#include <vector>\nint main() { return 0; }";
  for (const tag of ["cpp", "c++", "cc", "hpp", "h"]) {
    const html = highlight(src, tag);
    assert.match(html, /<span class="hljs-type">int<\/span>/, tag);
    assert.match(html, /<span class="hljs-keyword">return<\/span>/, tag);
    assert.match(html, /hljs-meta/, tag);
  }
});

test("every loader fetches a grammar that colours", async () => {
  for (const id of Object.keys(LOADERS)) {
    assert.equal(await loadLanguage(id), true, id);
    assert.ok(hasGrammar(id), id);
  }
});

test("bundled grammars colour straight away", () => {
  assert.match(highlight("def f(x):\n    return x", "python"), /<span class="hljs-keyword">def<\/span>/);
  assert.match(highlight("fn main() {}", "rs"), /hljs-keyword/);
});

test("unknown languages come back escaped, never thrown", async () => {
  assert.equal(highlight("<a> & b", "zig"), "&lt;a&gt; &amp; b");
  assert.equal(highlight("<a> & b", null), "&lt;a&gt; &amp; b");
  assert.equal(colour("x", "nope"), null);
  assert.equal(await loadLanguage("nope"), false);
  assert.equal(await loadLanguage(undefined), false);
});
