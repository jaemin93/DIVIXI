/**
 * One highlight.js instance shared by markdown rendering, the file viewer,
 * the editor and diffs. The grammars agents use most are bundled; the rest
 * load on first use, each as its own small chunk, so the start-up bundle
 * does not carry thirty languages nobody opened. Which names mean which
 * grammar lives in `languages.ts`; adding a language is a line there and a
 * loader here.
 */
import hljs from "highlight.js/lib/core";
import type { LanguageFn } from "highlight.js";

import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import shell from "highlight.js/lib/languages/shell";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

import { resolveLanguage } from "./languages.ts";

const BUNDLED: Record<string, LanguageFn> = { bash, css, diff, json, markdown, python, rust, shell, sql, typescript, xml, yaml };
for (const [id, fn] of Object.entries(BUNDLED)) hljs.registerLanguage(id, fn);
// HTML hands <script> to a grammar called "javascript"; TypeScript's covers it.
hljs.registerLanguage("javascript", typescript);

type Grammar = Promise<{ default: LanguageFn }>;

/** The grammars fetched on first use. The paths are literal so the bundler splits each into a chunk. */
export const LOADERS: Record<string, () => Grammar> = {
  c: () => import("highlight.js/lib/languages/c"),
  cmake: () => import("highlight.js/lib/languages/cmake"),
  cpp: () => import("highlight.js/lib/languages/cpp"),
  csharp: () => import("highlight.js/lib/languages/csharp"),
  dart: () => import("highlight.js/lib/languages/dart"),
  dockerfile: () => import("highlight.js/lib/languages/dockerfile"),
  elixir: () => import("highlight.js/lib/languages/elixir"),
  go: () => import("highlight.js/lib/languages/go"),
  graphql: () => import("highlight.js/lib/languages/graphql"),
  haskell: () => import("highlight.js/lib/languages/haskell"),
  ini: () => import("highlight.js/lib/languages/ini"),
  java: () => import("highlight.js/lib/languages/java"),
  kotlin: () => import("highlight.js/lib/languages/kotlin"),
  less: () => import("highlight.js/lib/languages/less"),
  lua: () => import("highlight.js/lib/languages/lua"),
  makefile: () => import("highlight.js/lib/languages/makefile"),
  objectivec: () => import("highlight.js/lib/languages/objectivec"),
  php: () => import("highlight.js/lib/languages/php"),
  powershell: () => import("highlight.js/lib/languages/powershell"),
  protobuf: () => import("highlight.js/lib/languages/protobuf"),
  r: () => import("highlight.js/lib/languages/r"),
  ruby: () => import("highlight.js/lib/languages/ruby"),
  scala: () => import("highlight.js/lib/languages/scala"),
  scss: () => import("highlight.js/lib/languages/scss"),
  swift: () => import("highlight.js/lib/languages/swift"),
};

/** Grammars on their way, so a burst of code blocks fetches each once. */
const loading = new Map<string, Promise<boolean>>();
const listeners = new Set<() => void>();

/** Called each time a grammar arrives, so what was drawn plain can be coloured. */
export function onGrammar(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Whether a grammar id can be coloured now (bundled, or loaded already). */
export function hasGrammar(id: string): boolean {
  return !!hljs.getLanguage(id);
}

/**
 * Make sure the grammar for `tag` (any name `languages.ts` knows) is
 * registered. Resolves `true` once it is; `false` for an unknown name or a
 * grammar that failed to load, which then stays plain without asking again.
 */
export function loadLanguage(tag: string | null | undefined): Promise<boolean> {
  const id = resolveLanguage(tag);
  if (!id) return Promise.resolve(false);
  if (hasGrammar(id)) return Promise.resolve(true);
  const load = LOADERS[id];
  if (!load) return Promise.resolve(false);
  let pending = loading.get(id);
  if (!pending) {
    pending = load().then(
      (mod) => {
        hljs.registerLanguage(id, mod.default);
        for (const fn of listeners) fn();
        return true;
      },
      () => false,
    );
    loading.set(id, pending);
  }
  return pending;
}

function escape(code: string): string {
  return code.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/**
 * Coloured HTML for `code` when its grammar is here; otherwise the grammar
 * is sent for and the text comes back as `null`, for the caller to show
 * plain until `onGrammar` says to draw again. Unknown names stay `null`.
 */
export function colour(code: string, tag: string | null | undefined): string | null {
  const id = resolveLanguage(tag);
  if (!id) return null;
  if (!hasGrammar(id)) {
    void loadLanguage(id);
    return null;
  }
  try {
    return hljs.highlight(code, { language: id, ignoreIllegals: true }).value;
  } catch {
    return null;
  }
}

/** HTML for `code`: coloured when the language is known and loaded, escaped otherwise. */
export function highlight(code: string, tag: string | null | undefined): string {
  return colour(code, tag) ?? escape(code);
}
