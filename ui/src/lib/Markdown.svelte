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
  import hljs from "highlight.js/lib/core";
  import DOMPurify from "dompurify";

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

  hljs.registerLanguage("bash", bash);
  hljs.registerLanguage("sh", bash);
  hljs.registerLanguage("css", css);
  hljs.registerLanguage("diff", diff);
  hljs.registerLanguage("json", json);
  hljs.registerLanguage("markdown", markdown);
  hljs.registerLanguage("md", markdown);
  hljs.registerLanguage("python", python);
  hljs.registerLanguage("py", python);
  hljs.registerLanguage("rust", rust);
  hljs.registerLanguage("rs", rust);
  hljs.registerLanguage("shell", shell);
  hljs.registerLanguage("sql", sql);
  hljs.registerLanguage("typescript", typescript);
  hljs.registerLanguage("ts", typescript);
  hljs.registerLanguage("javascript", typescript);
  hljs.registerLanguage("js", typescript);
  hljs.registerLanguage("svelte", xml);
  hljs.registerLanguage("html", xml);
  hljs.registerLanguage("xml", xml);
  hljs.registerLanguage("yaml", yaml);
  hljs.registerLanguage("yml", yaml);

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
