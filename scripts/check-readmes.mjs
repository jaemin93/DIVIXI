// README.md and README.ko.md are one document in two languages.
//
// Nothing else notices when they drift: a section added to one and not the
// other renders fine on its own, and a reader only finds out by comparing the
// two. This is the gate for that, in the same spirit as `check:i18n`.
//
//   npm run check:readme
//
// WHAT IT CHECKS, AND WHY ONLY THIS
//
// - **Heading structure** — the same headings, at the same levels, in the same
//   order. This is the drift that matters: a whole section present in one
//   language and missing from the other. The heading *text* differs by design
//   (it is translated), so only the level and the position are compared.
// - **Relative links resolve.** A link to a file that was moved or deleted is
//   the failure this repository has actually hit, and a translated copy
//   doubles the places it can hide.
// - **The language switcher** is present in both and points at the other file.
//   It is the one line that makes the pair discoverable; losing it in a
//   rewrite is easy and invisible.
// - **Fenced code blocks: the same number, and balanced.** An unclosed fence
//   swallows the rest of the page. The *contents* are deliberately not
//   compared: the commands are the same, but the comments beside them are
//   translated.
//
// It does NOT compare sentences, paragraphs, bullets, words or characters.
// Korean and English do not split a thought the same way — a bullet that is
// natural as one sentence in English may want two in Korean — and a gate on
// counts would turn every good translation into a failing build. Prose that
// says the same thing in a different shape is the translation working, not
// drift. Reviewers catch meaning; this catches structure.
//
// Paths are resolved from this file, not the working directory, so it can be
// run from anywhere. Two paths may be given instead, which is how this check
// is tested against a copy with a heading removed on purpose.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const en = process.argv[2] ?? path.join(root, "README.md");
const ko = process.argv[3] ?? path.join(root, "README.ko.md");

const read = (file) => {
  if (!fs.existsSync(file)) {
    console.log(`missing: ${file}`);
    process.exit(1);
  }
  return fs.readFileSync(file, "utf8");
};

const enText = read(en);
const koText = read(ko);
const problems = [];

/** Every heading, markdown or HTML, in the order it is written. */
function headings(src) {
  const out = [];
  for (const line of src.split("\n")) {
    const md = line.match(/^(#{1,6})\s+(.+)$/);
    if (md) out.push({ level: md[1].length, text: md[2].trim() });
    const html = line.match(/<h([1-6])[^>]*>(.*?)<\/h\1>/);
    if (html) out.push({ level: Number(html[1]), text: html[2].trim() });
  }
  return out;
}

const enHeads = headings(enText);
const koHeads = headings(koText);
console.log(`headings: README.md ${enHeads.length}, README.ko.md ${koHeads.length}`);
if (enHeads.length !== koHeads.length) {
  problems.push(`${enHeads.length} headings in en, ${koHeads.length} in ko`);
}
for (let i = 0; i < Math.max(enHeads.length, koHeads.length); i++) {
  const a = enHeads[i];
  const b = koHeads[i];
  if (a && b && a.level === b.level) continue;
  problems.push(
    `heading ${i + 1}: en ${a ? `h${a.level} "${a.text}"` : "(none)"}` +
      ` / ko ${b ? `h${b.level} "${b.text}"` : "(none)"}`,
  );
}

// Links. Relative targets are resolved from the repository root, where both
// READMEs live, so a copy under test still points at the real files.
function links(src) {
  const all = [];
  for (const m of src.matchAll(/\[[^\]]*\]\(([^)\s]+)\)/g)) all.push(m[1]);
  for (const m of src.matchAll(/(?:href|src)="([^"]+)"/g)) all.push(m[1]);
  return all;
}
for (const [name, src] of [["README.md", enText], ["README.ko.md", koText]]) {
  const relative = links(src).filter((t) => !/^(https?:|mailto:|#)/.test(t));
  const broken = relative.filter((t) => !fs.existsSync(path.join(root, t.split("#")[0])));
  console.log(`${name}: ${relative.length} relative links, ${broken.length} broken`);
  for (const t of broken) problems.push(`${name}: link to ${t} does not exist`);
}

// The language switcher. Each file names the other.
if (!enText.includes('href="README.ko.md"')) problems.push("README.md does not link to README.ko.md");
if (!koText.includes('href="README.md"')) problems.push("README.ko.md does not link to README.md");

// Fences: the same number of blocks, each one closed.
const fences = (src) => (src.match(/^```/gm) ?? []).length;
const enFences = fences(enText);
const koFences = fences(koText);
console.log(`fence markers: README.md ${enFences}, README.ko.md ${koFences}`);
if (enFences % 2) problems.push("README.md has an unclosed code fence");
if (koFences % 2) problems.push("README.ko.md has an unclosed code fence");
if (enFences !== koFences) problems.push(`${enFences / 2} code blocks in en, ${koFences / 2} in ko`);

if (problems.length) {
  for (const p of problems) console.log(p);
  console.log("the two READMEs do not match");
  process.exit(1);
}
console.log("README.md and README.ko.md match");
