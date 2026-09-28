// Documents that exist in two languages must stay the same document.
//
// Nothing else notices when they drift: a section added to one and not the
// other renders fine on its own, and a reader only finds out by comparing the
// two. This is the gate for that, in the same spirit as `check:i18n`.
//
//   npm run check:readme
//
// THE PAIRS
//
// Every `<name>.md` with a `<name>.ko.md` beside it. Add a translated
// document and add it to PAIRS; nothing else changes.
//
// WHAT IT CHECKS, AND WHY ONLY THIS
//
// - **Heading structure** — the same headings, at the same levels, in the same
//   order. This is the drift that matters: a whole section present in one
//   language and missing from the other. The heading *text* differs by design
//   (it is translated), so only the level and the position are compared.
// - **Relative links resolve**, from the directory the file lives in. A link
//   to a file that was moved or deleted is the failure this repository has
//   actually hit, and a translated copy doubles the places it can hide.
// - **The language switcher** is present in both and names the other file.
//   It is the one line that makes the pair discoverable; losing it in a
//   rewrite is easy and invisible. Markdown or HTML, either counts.
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
// run from anywhere. One pair may be given as two arguments instead, which is
// how this check is tested against a copy with a heading removed on purpose.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));

/** Every document that exists in both languages, as [English, Korean]. */
const PAIRS = [
  ["README.md", "README.ko.md"],
  ["docs/divixi-server.md", "docs/divixi-server.ko.md"],
];

const pairs = process.argv.length > 3 ? [[process.argv[2], process.argv[3]]] : PAIRS;
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

/** Every link target: markdown `[…](target)` and HTML `href=`/`src=`. */
function links(src) {
  const all = [];
  for (const m of src.matchAll(/\[[^\]]*\]\(([^)\s]+)\)/g)) all.push(m[1]);
  for (const m of src.matchAll(/(?:href|src)="([^"]+)"/g)) all.push(m[1]);
  return all;
}

for (const [enRel, koRel] of pairs) {
  const files = [enRel, koRel].map((rel) => {
    const file = path.isAbsolute(rel) ? rel : path.join(root, rel);
    if (!fs.existsSync(file)) {
      console.log(`missing: ${rel}`);
      process.exit(1);
    }
    return { rel, file, src: fs.readFileSync(file, "utf8") };
  });
  const [en, ko] = files;
  console.log(`\n${en.rel} / ${ko.rel}`);

  // Headings: same levels, same order.
  const enHeads = headings(en.src);
  const koHeads = headings(ko.src);
  console.log(`  headings: ${enHeads.length} / ${koHeads.length}`);
  if (enHeads.length !== koHeads.length) {
    problems.push(`${en.rel}: ${enHeads.length} headings, ${ko.rel}: ${koHeads.length}`);
  }
  for (let i = 0; i < Math.max(enHeads.length, koHeads.length); i++) {
    const a = enHeads[i];
    const b = koHeads[i];
    if (a && b && a.level === b.level) continue;
    problems.push(
      `${en.rel} heading ${i + 1}: ${a ? `h${a.level} "${a.text}"` : "(none)"}` +
        ` / ${ko.rel}: ${b ? `h${b.level} "${b.text}"` : "(none)"}`,
    );
  }

  // Links, resolved from the directory the document lives in.
  for (const f of files) {
    const relative = links(f.src).filter((t) => !/^(https?:|mailto:|#)/.test(t));
    const broken = relative.filter(
      (t) => !fs.existsSync(path.join(path.dirname(f.file), t.split("#")[0])),
    );
    console.log(`  ${f.rel}: ${relative.length} relative links, ${broken.length} broken`);
    for (const t of broken) problems.push(`${f.rel}: link to ${t} does not exist`);
  }

  // The language switcher: each file names the other, by file name.
  for (const [self, other] of [[en, ko], [ko, en]]) {
    const name = path.basename(other.rel);
    if (!links(self.src).some((t) => path.basename(t.split("#")[0]) === name)) {
      problems.push(`${self.rel} does not link to ${name}`);
    }
  }

  // Fences: the same number of blocks, each one closed.
  const fences = (src) => (src.match(/^```/gm) ?? []).length;
  const enFences = fences(en.src);
  const koFences = fences(ko.src);
  console.log(`  fence markers: ${enFences} / ${koFences}`);
  if (enFences % 2) problems.push(`${en.rel} has an unclosed code fence`);
  if (koFences % 2) problems.push(`${ko.rel} has an unclosed code fence`);
  if (enFences !== koFences) {
    problems.push(`${enFences / 2} code blocks in ${en.rel}, ${koFences / 2} in ${ko.rel}`);
  }
}

console.log("");
if (problems.length) {
  for (const p of problems) console.log(p);
  console.log("the translated documents do not match");
  process.exit(1);
}
console.log(`${pairs.length} pair(s) match`);
