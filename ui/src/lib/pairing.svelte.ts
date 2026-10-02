import { invoke } from "./ipc.svelte";
import { encodeQr, type Qr } from "./qr";
import { clock, codeState, secondsLeft, type CodeState } from "./countdown";

/**
 * The pairing code currently on offer.
 *
 * Module scope, so the Overview card and the rail's dialog show the *same*
 * code and hiding one does not throw it away. Minting on every open left a
 * trail of live codes — each good for five minutes and each a way in — when
 * one was wanted. Fewer live codes is the better property, and re-showing the
 * one that is still good is also what somebody expects after closing a panel
 * by accident.
 */

export type PairLink = { url: string; expires: number; days: number };

const now = () => Math.floor(Date.now() / 1000);

class Pairing {
  link = $state<PairLink | null>(null);
  qr = $state<Qr | null>(null);
  error = $state("");
  busy = $state(false);
  /**
   * The time the countdown is drawn against, moved on by `tick`. A state and
   * not `Date.now()` in `left`: reading the clock is not something a view can
   * depend on, so the countdown was drawn once and stayed at 5:00.
   */
  now = $state(now());

  /** Seconds before this code stops working; zero when there is none. */
  get left(): number {
    return this.link ? secondsLeft(this.link.expires, this.now) : 0;
  }

  /** `m:ss`, for the countdown beside the code. */
  get clock(): string {
    return clock(this.left);
  }

  /** `expired` stays until somebody asks for a new code: none is made on its own. */
  get state(): CodeState {
    return codeState(this.link?.expires ?? null, this.now);
  }

  /**
   * A code to show: the one in hand, live or expired, otherwise a new one.
   * An expired one is shown as expired; only its button makes another.
   */
  async ensure(): Promise<void> {
    // Nothing ticks while no code is on screen, so `now` may be old.
    this.now = now();
    if (this.state !== "none" || this.busy) return;
    await this.mint();
  }

  async mint(): Promise<void> {
    this.busy = true;
    this.error = "";
    this.forget();
    try {
      const link = await invoke<PairLink>("phone_pair_link");
      // Drawn before it is shown: a payload that cannot be encoded has to
      // fail here and not as an empty square on the screen.
      this.qr = encodeQr(link.url);
      this.now = now();
      this.link = link;
    } catch (err) {
      this.error = String(err);
    } finally {
      this.busy = false;
    }
  }

  /**
   * Move the countdown on. At zero the code turns `expired` and the view
   * takes it off the screen: a dead code looks exactly like a live one, and
   * the only way to tell is to fail at the phone.
   */
  tick(): void {
    this.now = now();
  }

  forget(): void {
    this.link = null;
    this.qr = null;
  }
}

export const pairing = new Pairing();
