import { invoke } from "./ipc.svelte";
import { encodeQr, type Qr } from "./qr";

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

  /** Seconds before this code stops working; zero when there is none. */
  get left(): number {
    return this.link ? Math.max(0, this.link.expires - now()) : 0;
  }

  /** `m:ss`, for the countdown beside the code. */
  get clock(): string {
    const l = this.left;
    return `${Math.floor(l / 60)}:${String(l % 60).padStart(2, "0")}`;
  }

  /** A code to show: the one in hand if it is still good, otherwise a new one. */
  async ensure(): Promise<void> {
    if (this.link && this.left > 0) return;
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
      this.link = link;
    } catch (err) {
      this.error = String(err);
    } finally {
      this.busy = false;
    }
  }

  /**
   * Take an expired code off the screen. A dead code looks exactly like a
   * live one, and the only way to tell is to fail at the phone.
   */
  tick(): void {
    if (this.link && this.left === 0) this.forget();
  }

  forget(): void {
    this.link = null;
    this.qr = null;
  }
}

export const pairing = new Pairing();
