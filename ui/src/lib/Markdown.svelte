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

  const marked = new Marked(
    { gfm: true, breaks: true },
    markedHighlight({
      langPrefix: "hljs language-",
      highlight(code, lang) {
        const language = hljs.getLanguage(lang) ? lang : undefined;
        return language ? hljs.highlight(code, { language }).value : hljs.highlightAuto(code).value;
      },
    }),
  );

  let { source }: { source: string } = $props();

  const html = $derived(DOMPurify.sanitize(marked.parse(source, { async: false }) as string));
</script>

<div class="md">{@html html}</div>
