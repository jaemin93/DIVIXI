<script lang="ts">
  /**
   * Markdown as agents write it: GFM (tables, task lists, strikethrough),
   * fenced code with syntax colours, sanitised before it touches the DOM.
   * Re-renders on every chunk while streaming; marked is fast enough that
   * this is cheaper than being clever about partial fences.
   *
   * The same stack Kiro Crew's dashboard uses, in Svelte form: a markdown
   * parser, highlight.js, DOMPurify.
   */
  import { Marked } from "marked";
  import { markedHighlight } from "marked-highlight";
  import DOMPurify from "dompurify";
  import { hljs } from "./highlight";
  import { store } from "./store.svelte";

  // A picture from the web cannot load here (the page allows none), and a
  // broken image says nothing: it becomes a link to open in the browser.
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName !== "IMG") return;
    const src = node.getAttribute("src") ?? "";
    if (!/^https?:/i.test(src)) return;
    const a = document.createElement("a");
    a.setAttribute("href", src);
    a.textContent = node.getAttribute("alt") || src;
    node.replaceWith(a);
  });

  const marked = new Marked(
    { gfm: true, breaks: true },
    markedHighlight({
      langPrefix: "hljs language-",
      highlight(code, lang) {
        // Only a tagged fence is coloured: guessing the language runs every
        // grammar over the block, again on each streamed chunk.
        // Returning the text unchanged tells marked-highlight to escape it itself.
        return hljs.getLanguage(lang) ? hljs.highlight(code, { language: lang, ignoreIllegals: true }).value : code;
      },
    }),
  );

  let {
    source,
    base = "",
  }: {
    source: string;
    /** The folder (track-relative) a document's relative links start from. */
    base?: string;
  } = $props();

  /** `a/b/../c` → `a/c`; `null` when it climbs out of the track. */
  function normal(path: string): string | null {
    const out: string[] = [];
    for (const part of path.split("/")) {
      if (!part || part === ".") continue;
      if (part === "..") {
        if (!out.length) return null;
        out.pop();
      } else out.push(part);
    }
    return out.join("/");
  }

  /** A link never takes the app's window away: web links open in the
   *  browser, file links in the side panel, anything else does nothing. */
  function onClick(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest?.("a");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") ?? "";
    if (!href || href.startsWith("#")) return;
    if (/^https?:/i.test(href)) {
      void store.openUrl(href);
      return;
    }
    // Some other scheme (mailto:, file:, …): not followed. A drive letter is a path.
    if (/^[a-z][a-z0-9+.-]*:/i.test(href) && !/^[a-z]:[\\/]/i.test(href)) return;
    let path = href.split("#")[0].split("?")[0];
    try {
      path = decodeURIComponent(path);
    } catch {
      // Keep it as written.
    }
    path = path.replace(/\\/g, "/");
    const absolute = path.startsWith("/") || /^[a-z]:\//i.test(path);
    const rel = absolute ? store.relativeToTrack(path) : normal(base ? `${base}/${path}` : path);
    if (rel) void store.openFile(rel);
  }

  const html = $derived(DOMPurify.sanitize(marked.parse(source, { async: false }) as string));
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="md" onclick={onClick}>{@html html}</div>
