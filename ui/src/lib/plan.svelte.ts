import { invoke, listen } from "./ipc.svelte";
import { store, type ArtifactInfo, type Routine } from "./store.svelte";
import { emptyPlan, instructionChanged, unsaved, type PlanDoc, type PlanEvent } from "./planGraph";

/**
 * A routine's plan on screen (src-tauri/src/plan.rs keeps it): its steps
 * as a graph, the step picked, its saved versions, and the moves that
 * change it. The conversation with the plan's agent is the store's artifact
 * conversation (store.planInfo, opened from the robot toggle), as a
 * design's is; this is the plan beside it.
 */

/** What `routine_plan_open` answers. */
type Opened = { routine: Routine; plan: ArtifactInfo; doc: PlanDoc };

class Plans {
  /** The routine whose plan this is, and whether its plan is on screen. */
  routine = $state("");
  open = $state(false);
  doc = $state<PlanDoc>(emptyPlan());
  /** The plan's artifact id, once known. */
  id = $state("");
  /** The step whose drawer is open. */
  picked = $state("");
  busy = $state(false);
  /** The run whose step results the plan shows; empty is the newest that has any. */
  resultRun = $state("");
  /** Bumped when the graph should frame the whole plan again (after a layout). */
  refit = $state(0);

  /** The routine on screen, if it is the one this plan belongs to. */
  get routineOnScreen(): Routine | undefined {
    return store.routines.find((r) => r.id === this.routine);
  }

  get changed(): boolean {
    const r = this.routineOnScreen;
    return !!r && instructionChanged(this.doc, r.instruction);
  }

  /** The working plan differs from the version runs are given. */
  get unsaved(): boolean {
    return unsaved(this.doc);
  }

  /** The version runs are given now, if any. */
  get lastSaved(): number | undefined {
    return this.doc.saved?.at(-1)?.n;
  }

  /** Leave the plan when another routine is opened. */
  follow(routine: string) {
    if (routine === this.routine) return;
    this.routine = routine;
    this.open = false;
    this.id = "";
    this.doc = emptyPlan();
    this.picked = "";
    this.resultRun = "";
    store.planInfo = null;
    store.planSelected = [];
  }

  /**
   * Open the routine's plan, made when it has none. Nothing is sent to its
   * agent: the human asks it, from the robot panel, when they want to.
   */
  async openFor(routine: string): Promise<boolean> {
    this.follow(routine);
    this.busy = true;
    try {
      const got = await invoke<Opened>("routine_plan_open", { routine, agent: null });
      if (this.routine !== routine) return false;
      this.id = got.plan.id;
      this.doc = got.doc;
      this.open = true;
      store.planInfo = got.plan;
      // The routine now carries its plan's id; the list hears it too.
      store.routines = store.routines.map((r) => (r.id === routine ? got.routine : r));
      void store.refreshArtifactSession();
      return true;
    } catch (err) {
      store.lastError = String(err);
      return false;
    } finally {
      this.busy = false;
    }
  }

  close() {
    this.open = false;
    this.picked = "";
  }

  /** A change the core sent: taken when it is this plan's and not older. */
  take(e: PlanEvent) {
    if (e.id !== this.id) return;
    if (e.doc.version < this.doc.version) return;
    this.doc = e.doc;
    if (this.picked && !e.doc.steps.some((s) => s.id === this.picked)) this.picked = "";
    store.planSelected = store.planSelected.filter((id) => e.doc.steps.some((s) => s.id === id));
  }

  async apply(ops: Record<string, unknown>[]): Promise<Record<string, unknown>[]> {
    if (!this.id) return [];
    try {
      const out = await invoke<Record<string, unknown>[]>("plan_apply", { id: this.id, ops });
      const refused = out.find((r) => r.ok === false);
      if (refused) store.lastError = String(refused.error ?? "refused");
      return out;
    } catch (err) {
      store.lastError = String(err);
      return [];
    }
  }

  async review(changes: number[], keep: boolean) {
    if (!this.id || !changes.length) return;
    try {
      await invoke("plan_review", { id: this.id, changes, keep });
    } catch (err) {
      store.lastError = String(err);
    }
  }

  async undo(again = false) {
    if (!this.id) return;
    try {
      await invoke<boolean>("plan_undo", { id: this.id, again });
    } catch (err) {
      store.lastError = String(err);
    }
  }

  /**
   * Lay every step out again. A field still being typed in is written
   * first, so the layout is of the plan as it now reads; then the graph
   * frames the whole plan, so what moved is in view.
   */
  async autoLayout() {
    (document.activeElement as HTMLElement | null)?.blur?.();
    // The field's write goes out on blur; let it land first.
    await new Promise((r) => setTimeout(r, 0));
    await this.apply([{ op: "auto_layout" }]);
    setTimeout(() => this.refit++, 60);
  }

  /** Save the working plan as the version runs are given. */
  async save(note: string): Promise<boolean> {
    const out = await this.apply([{ op: "save", note }]);
    return out[0]?.ok === true;
  }

  /** Bring a saved version back into the working plan. */
  async openVersion(n: number) {
    await this.apply([{ op: "open", n }]);
    setTimeout(() => this.refit++, 60);
  }

  /** Pick a step: its drawer opens, and it goes with the next message. */
  pick(id: string, add = false) {
    if (add) {
      const has = store.planSelected.includes(id);
      store.planSelected = has ? store.planSelected.filter((x) => x !== id) : [...store.planSelected, id];
      this.picked = store.planSelected.length === 1 ? store.planSelected[0] : "";
      return;
    }
    store.planSelected = id ? [id] : [];
    this.picked = id;
  }

  /** The plan now answers to the instruction as it reads (the band goes). */
  async restamp() {
    const r = this.routineOnScreen;
    if (r) await this.apply([{ op: "restamp", instruction: r.instruction }]);
  }

  /** Take the agent's proposed instruction into the routine. */
  async takeProposal() {
    const r = this.routineOnScreen;
    const text = this.doc.proposal?.text;
    if (!r || !text) return;
    const ok = await store.saveRoutine(r.id, { instruction: text });
    if (!ok) return;
    await this.apply([{ op: "dismiss_proposal" }, { op: "restamp", instruction: text }]);
  }
}

export const plans = new Plans();

/** Subscribe once. */
export async function connectPlans() {
  await listen<PlanEvent>("plan", (e) => plans.take(e.payload));
}
