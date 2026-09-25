<script lang="ts">
  import { highlight } from "./highlight";

  /**
   * A plain-text editor that looks like the file viewer: the highlighted
   * text drawn underneath, a transparent textarea over it taking the typing,
   * both scrolling together. The textarea keeps the browser's own undo;
   * edits go through `insertText` so Tab, Enter and the like are undoable
   * too. Tab indents (Shift+Tab outdents) in the file's own style, Enter
   * keeps the line's indentation, Ctrl/Cmd+S saves.
   */
  let {
    value,
    language,
    onchange,
    onsave,
  }: {
    value: string;
    language: string | null;
    onchange: (text: string) => void;
    onsave: () => void;
  } = $props();

  let area = $state<HTMLTextAreaElement>();
  /** What is drawn: follows `value` once a frame, so typing never waits on colouring. */
  let drawn = $state("");
  let pending = 0;

  $effect(() => {
    const v = value;
    // A value from outside (another file, a reload, a save) replaces the text.
    if (area && area.value !== v) area.value = v;
    if (!pending) {
      pending = requestAnimationFrame(() => {
        pending = 0;
        drawn = area?.value ?? v;
      });
    }
    return () => {
      cancelAnimationFrame(pending);
      pending = 0;
    };
  });

  /** Past this, colouring every keystroke costs more than it gives. */
  const PLAIN_OVER = 300_000;
  const html = $derived(drawn.length > PLAIN_OVER ? highlight(drawn, null) : highlight(drawn, language));

  /** One indentation step as the file writes it: a tab, or its first indent's spaces. */
  const unit = $derived.by(() => {
    const m = /^([ \t]+)\S/m.exec(value);
    if (!m) return "    ";
    if (m[1].startsWith("\t")) return "\t";
    return m[1].length === 2 ? "  " : "    ";
  });

  /** Replace the selection, undoably. */
  function insert(text: string) {
    const el = area!;
    el.focus();
    if (!document.execCommand("insertText", false, text)) {
      el.setRangeText(text, el.selectionStart, el.selectionEnd, "end");
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }
  }

  function keydown(e: KeyboardEvent) {
    const el = area!;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      onsave();
      return;
    }
    if (e.isComposing) return;
    const v = el.value;
    const s = el.selectionStart;
    const end = el.selectionEnd;
    if (e.key === "Tab") {
      e.preventDefault();
      const lineStart = v.lastIndexOf("\n", s - 1) + 1;
      const multiline = v.slice(s, end).includes("\n");
      if (!multiline && !e.shiftKey) {
        insert(unit);
        return;
      }
      // Indent or outdent every line the selection touches.
      const blockEnd = multiline ? end : v.indexOf("\n", s) === -1 ? v.length : v.indexOf("\n", s);
      const block = v.slice(lineStart, blockEnd);
      // Outdenting takes one step off: a tab, or up to one step of spaces.
      const outdent = unit === "\t" ? /^\t/gm : new RegExp(`^(\\t| {1,${unit.length}})`, "gm");
      const out = e.shiftKey ? block.replace(outdent, "") : block.replace(/^/gm, unit);
      if (out === block) return;
      el.setSelectionRange(lineStart, blockEnd);
      insert(out);
      if (multiline) el.setSelectionRange(lineStart, lineStart + out.length);
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      const lineStart = v.lastIndexOf("\n", s - 1) + 1;
      const indent = /^[ \t]*/.exec(v.slice(lineStart, s))?.[0] ?? "";
      insert(`\n${indent}`);
    }
  }
</script>

<div class="scroll">
  <div class="ed">
    <pre class="hl hljs" aria-hidden="true">{@html html}{"\n"}</pre>
    <textarea
      bind:this={area}
      oninput={(e) => onchange(e.currentTarget.value)}
      onscroll={(e) => {
        // It scrolls itself when the caret runs past the drawn text for a
        // frame; the box around it does the scrolling, so it stays aligned.
        e.currentTarget.scrollTop = 0;
        e.currentTarget.scrollLeft = 0;
      }}
      onkeydown={keydown}
      spellcheck="false"
      autocomplete="off"
      autocapitalize="off"
      wrap="off"
      aria-label="editor"
    ></textarea>
  </div>
</div>

<style>
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  /* The drawn text sizes the box; the textarea lies exactly over it. */
  .ed {
    position: relative;
    min-width: 100%;
    width: max-content;
    min-height: 100%;
  }

  .hl,
  textarea {
    margin: 0;
    padding: 12px 14px;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.6;
    white-space: pre;
    tab-size: 4;
    letter-spacing: normal;
    border: 0;
    box-sizing: border-box;
  }

  .hl {
    color: var(--body);
    background: transparent;
    pointer-events: none;
    min-height: 100%;
  }

  textarea {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    resize: none;
    overflow: hidden;
    background: transparent;
    color: transparent;
    caret-color: var(--hi);
    outline: none;
  }

  textarea::selection {
    background: var(--accbg);
    color: transparent;
  }
</style>
