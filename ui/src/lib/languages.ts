/**
 * Which highlight.js grammar a fence tag or a file name means. Pure data and
 * lookups, no highlight.js here: `highlight.ts` owns the grammars and loads
 * each one the first time it is asked for.
 *
 * Every id is the name of a grammar file under `highlight.js/lib/languages`.
 * The names after it are what people write: fence tags (```c++) and file
 * extensions share one list, since they are mostly the same words.
 */
export const LANGUAGES: Record<string, readonly string[]> = {
  bash: ["sh", "bash", "zsh", "ksh", "shell", "shellscript"],
  c: ["c"],
  cmake: ["cmake"],
  cpp: ["cpp", "c++", "cc", "cxx", "h", "hpp", "hh", "hxx", "h++", "ipp", "tpp", "inl", "ino"],
  csharp: ["cs", "csharp", "c#", "csx"],
  css: ["css"],
  dart: ["dart"],
  diff: ["diff", "patch"],
  dockerfile: ["dockerfile", "docker", "containerfile"],
  elixir: ["ex", "exs", "elixir"],
  go: ["go", "golang"],
  graphql: ["graphql", "gql"],
  haskell: ["hs", "lhs", "haskell"],
  ini: ["ini", "toml", "conf", "cfg", "properties", "editorconfig", "gitconfig"],
  java: ["java"],
  json: ["json", "jsonc", "json5", "jsonl", "ndjson", "geojson", "webmanifest"],
  kotlin: ["kt", "kts", "kotlin"],
  less: ["less"],
  lua: ["lua"],
  makefile: ["make", "makefile", "mk", "mak"],
  markdown: ["md", "markdown", "mdx", "mkd"],
  objectivec: ["m", "mm", "objc", "objectivec", "objective-c", "obj-c"],
  php: ["php", "phtml"],
  powershell: ["ps1", "psm1", "psd1", "powershell", "pwsh"],
  protobuf: ["proto", "protobuf"],
  python: ["py", "python", "pyi", "pyw", "python3", "py3", "gyp"],
  r: ["r", "rmd"],
  ruby: ["rb", "ruby", "gemspec", "rake"],
  rust: ["rs", "rust"],
  scala: ["scala", "sc", "sbt"],
  scss: ["scss"],
  shell: ["console", "shellsession", "terminal"],
  sql: ["sql", "psql", "mysql", "sqlite", "plsql"],
  swift: ["swift"],
  // TypeScript's grammar is JavaScript's plus types, JSX included: one
  // grammar colours both well enough, and it is already loaded.
  typescript: ["ts", "tsx", "mts", "cts", "typescript", "js", "jsx", "mjs", "cjs", "javascript", "node"],
  xml: ["xml", "html", "htm", "xhtml", "svg", "svelte", "vue", "xsd", "xsl", "xslt", "plist", "csproj", "props", "targets", "resx"],
  yaml: ["yaml", "yml"],
};

/** Name → grammar id, built once from the table above. A listed name wins
 *  over a grammar's own id: ```shell means a script (bash), not a session. */
const BY_NAME: ReadonlyMap<string, string> = new Map([
  ...Object.keys(LANGUAGES).map((id) => [id, id] as const),
  ...Object.entries(LANGUAGES).flatMap(([id, names]) => names.map((n) => [n, id] as const)),
]);

/** Whole file names that say the language when the extension does not. */
const FILE_NAMES: ReadonlyMap<string, string> = new Map([
  ["dockerfile", "dockerfile"],
  ["containerfile", "dockerfile"],
  ["makefile", "makefile"],
  ["gnumakefile", "makefile"],
  ["cmakelists.txt", "cmake"],
  ["gemfile", "ruby"],
  ["rakefile", "ruby"],
  ["podfile", "ruby"],
  ["vagrantfile", "ruby"],
  [".bashrc", "bash"],
  [".bash_profile", "bash"],
  [".zshrc", "bash"],
  [".profile", "bash"],
  [".gitconfig", "ini"],
  [".editorconfig", "ini"],
  ["cargo.lock", "ini"],
]);

/**
 * The grammar id for a fence's info string (```c++ title="x" → `cpp`), or
 * `null` when it names nothing known. Case and a leading dot or brace
 * (```{.python}) do not matter.
 */
export function resolveLanguage(tag: string | null | undefined): string | null {
  if (!tag) return null;
  const word = tag.trim().split(/[\s,{}]+/).find(Boolean) ?? "";
  const name = word.toLowerCase().replace(/^\./, "");
  return BY_NAME.get(name) ?? null;
}

/**
 * The grammar id for a file, by its whole name first (Dockerfile,
 * CMakeLists.txt) and then its extension; `null` leaves it plain text.
 * Takes a bare name or a path with either slash.
 */
export function languageForFile(path: string | null | undefined): string | null {
  if (!path) return null;
  const name = (path.split(/[\\/]/).pop() ?? "").toLowerCase();
  if (!name) return null;
  const whole = FILE_NAMES.get(name);
  if (whole) return whole;
  // Dockerfile.dev, Makefile.win and the like.
  const stem = name.split(".")[0];
  if (stem === "dockerfile" || stem === "containerfile") return "dockerfile";
  if (stem === "makefile") return "makefile";
  const dot = name.lastIndexOf(".");
  if (dot < 0) return null;
  return BY_NAME.get(name.slice(dot + 1)) ?? null;
}
