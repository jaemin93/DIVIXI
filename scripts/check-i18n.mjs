// ko and en must hold exactly the same keys.
//
// `t()` is typed `keyof typeof ko`, so `npm run check` already catches a key
// that only en is missing — but only where the dictionaries are read as
// TypeScript. This reads them as text, so it also catches the reverse (a key
// en has and ko does not, which `Record<Key, string>` allows), a key written
// twice in one dictionary (the last one silently wins), and it says the
// counts out loud, which is what a person wants to see after adding a dozen
// strings.
//
//   npm run check:i18n
//
// Paths are resolved from this file, not the working directory, so it can be
// run from anywhere.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

// A path may be given instead, which is how this check is tested against a
// copy with a key missing on purpose.
const file = process.argv[2] ?? fileURLToPath(new URL("../ui/src/lib/i18n.svelte.ts", import.meta.url));
const src = fs.readFileSync(file, "utf8");

/** The keys of one dictionary, in the order they are written. */
function keys(name) {
  // `const ko = {` and `const en: Record<Key, string> = {`.
  const open = src.search(new RegExp(`^const ${name}\\b[^=]*= \\{$`, "m"));
  if (open < 0) throw new Error(`no ${name} dictionary in ${file}`);
  // ko ends `} as const;`, en a plain `};`.
  const after = src.slice(open).search(/^\}( as const)?;$/m);
  if (after < 0) throw new Error(`the ${name} dictionary does not end`);
  return [...src.slice(open, open + after).matchAll(/^ {2}"([^"]+)":/gm)].map((m) => m[1]);
}

const ko = keys("ko");
const en = keys("en");
const missing = (a, b) => a.filter((k) => !b.includes(k));
const twice = (list) => [...new Set(list.filter((k, i) => list.indexOf(k) !== i))];

const problems = [
  ["only in ko", missing(ko, en)],
  ["only in en", missing(en, ko)],
  ["written twice in ko", twice(ko)],
  ["written twice in en", twice(en)],
];

console.log(`ko ${ko.length} keys, en ${en.length} keys`);
for (const [what, list] of problems) if (list.length) console.log(`${what}: ${list.join(", ")}`);

if (problems.some(([, list]) => list.length)) {
  console.log("the two dictionaries do not match");
  process.exit(1);
}
console.log("ko and en match");
