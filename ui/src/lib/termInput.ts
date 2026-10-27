/**
 * What a terminal types, on its way to the shell, in the order it was typed.
 *
 * Two things can put it out of order, and both bit Korean typing: "대단히
 * 감사합니다." arrived as "대단히 감사합니.다" -- the last syllable, still being
 * composed when "." was pressed, landed after it.
 *
 * - The way out. Each `term_write` is a command of its own, and for a remote
 *   instance each is an HTTP request of its own (src-tauri/src/remote/client.rs),
 *   which the instance runs on a thread of its own (remote/bridge.rs). Two
 *   sent together race, and the second can be written first. [`InputQueue`]
 *   keeps one request in flight per terminal; what is typed meanwhile goes
 *   with the next one, together.
 * - xterm.js itself. It sends a finished composition from a timer (it reads
 *   the text from its textarea once the IME is done with it), so a key the
 *   IME passes through as a keypress right after is sent first -- and, as
 *   xterm leaves that keypress to reach the textarea too, sent again by the
 *   timer: "합니.다.". [`CompositionGuard`] keeps xterm from handling that
 *   keypress: the character goes into the textarea, after the composed
 *   text, and the timer sends both, once and in order.
 */

/** One terminal's input, sent one request at a time. */
export class InputQueue {
  private readonly send: (data: string) => Promise<unknown>;
  private pending = "";
  private sending: Promise<void> | null = null;

  /** `send` delivers one piece to the shell; the next is not sent before it settles. */
  constructor(send: (data: string) => Promise<unknown>) {
    this.send = send;
  }

  /** Queue what was typed; sent at once when nothing is on its way. */
  push(data: string): void {
    if (!data) return;
    this.pending += data;
    this.sending ??= this.drain();
  }

  /** Settles once everything queued so far has been sent. */
  idle(): Promise<void> {
    return this.sending ?? Promise.resolve();
  }

  private async drain(): Promise<void> {
    try {
      while (this.pending) {
        const data = this.pending;
        this.pending = "";
        try {
          await this.send(data);
        } catch {
          // The shell is gone; its exit is reported on its own.
        }
      }
    } finally {
      this.sending = null;
    }
  }
}

/**
 * From a composition's end until xterm.js has sent its text.
 *
 * Wire `ended` to the textarea's `compositionend` (after xterm's own
 * listener, so the timer it schedules runs after xterm's), `sent` to
 * xterm's `onData`, and `allows` into `attachCustomKeyEventHandler`.
 */
export class CompositionGuard {
  private readonly later: (fn: () => void) => void;
  private committing = false;

  /** `later` runs a function after the timers already scheduled (a 0 ms timeout). */
  constructor(later: (fn: () => void) => void = (fn) => setTimeout(fn, 0)) {
    this.later = later;
  }

  /** A composition ended: xterm will send its text from a timer. */
  ended(): void {
    this.committing = true;
    this.later(() => (this.committing = false));
  }

  /** xterm sent something: the composed text is out (or was nothing). */
  sent(): void {
    this.committing = false;
  }

  /**
   * Whether xterm may handle this key event. A keypress while the composed
   * text is still to be sent is left to the browser, which puts its
   * character in the textarea after that text; xterm sends the two together.
   */
  allows(e: Pick<KeyboardEvent, "type">): boolean {
    return !(this.committing && e.type === "keypress");
  }
}
