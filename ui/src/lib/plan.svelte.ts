import { invoke, listen } from "./ipc.svelte";
import { store, type ArtifactInfo, type Routine } from "./store.svelte";
import { t } from "./i18n.svelte";
import { emptyPlan, instructionChanged, type PlanDoc, type PlanEvent } from "./planGraph";

/**
 * A routine's plan on screen (src-tauri/src/plan.rs keeps it): its steps,
 * the graph or the list they are shown as, the step picked, and the moves
 * that change it. The conversation with the plan's agent is the store's
 * artifact conversation (store.planInfo, store.planChatOpen), as a design's
 * is; this is the plan beside it.
 */

/** What `routine_plan_open` answers. */
type Opened = { routine: Routine; plan: ArtifactInfo; doc: PlanDoc };

export type PlanShow = "graph" | "list";

/** A phone-sized screen gets the list: the graph has no room there. */
const narrowQuery = typeof matchMedia === "function" ? matchMedia("(max-width: 640px)") : null;

class Plans {
  /** The routine whose plan this is, and whether its plan is on screen. */
  routine = $state("");
  open = $state(false);
  doc = $state<PlanDoc>(emptyPlan());
  /** The plan's artifact id, once known. */
  id = $state("");
  /** The graph or the list, as the human last chose (a phone always has the list). */
  chosen = $state<PlanShow>("graph");
  narrow = $state(narrowQuery?.matches ?? false);
  /** The step whose drawer is open. */
  picked = $state("");
  busy = $state(false);
  /** The "plan again" menu: bring in line with the instruction, or start over. */
  menu = $state(false);
  /** The run whose step results the plan shows; empty is the newest that has any. */
  resultRun = $state("");

  constructor() {
    narrowQuery?.addEventListener("change", (e) => (this.narrow = e.matches));
  }

  get show(): PlanShow {
    return this.narrow ? "list" : this.chosen;
  }

  setShow(v: PlanShow) {
    this.chosen = v;
    void invoke("set_setting", { key: "plan_show", value: v }).catch(() => {});
  }

  async restoreShow() {
    try {
      const v = await invoke<string | null>("get_setting", { key: "plan_show" });
      if (v === "graph" || v === "list") this.chosen = v;
    } catch {
      // A preference; the graph it is.
    }
  }

  /** The routine on screen, if it is the one this plan belongs to. */
  get routineOnScreen(): Routine | undefined {
    return store.routines.find((r) => r.id === this.routine);
  }

  get changed(): boolean {
    const r = this.routineOnScreen;
    return !!r && instructionChanged(this.doc, r.instruction);
  }

  /** Leave the plan when another routine is opened. */
  follow(routine: string) {
    if (routine === this.routine) return;
    this.routine = routine;
    this.open = false;
    this.id = "";
    this.doc = emptyPlan();
    this.picked = "";
    this.menu = false;
    this.resultRun = "";
    store.planInfo = null;
    store.planChatOpen = false;
    store.planSelected = [];
  }

  /**
   * Open the routine's plan, made when it has none. The plan's agent starts
   * as the one last chosen for a plan (the core remembers), never the
   * routine's own by intent; the routine's is only the very first fallback.
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
      store.planChatOpen = true;
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
    store.planChatOpen = false;
  }

  setChat(open: boolean) {
    store.planChatOpen = open && !!store.planInfo;
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

  /** A message to the plan's agent, as if typed in its composer. */
  private async say(text: string) {
    if (!store.planInfo) return;
    store.planChatOpen = true;
    await store.send(text);
  }

  /** The first draft: the plan's agent splits the instruction into steps. */
  async draft() {
    await this.say(t("plan.ask.draft"));
  }

  /** Bring the plan in line with the instruction; what changes comes as suggestions. */
  async merge() {
    // The plan answers to the instruction as it reads now; the band goes,
    // and what the agent changes to match comes as suggestions.
    await this.restamp();
    await this.say(t("plan.ask.merge"));
  }

  /** Keep the plan in the history, empty it, and ask for a new draft. */
  async startOver() {
    await this.apply([{ op: "reset" }]);
    await this.restamp();
    await this.draft();
  }

  /** Bring back an earlier plan from the history (the current one takes its place there). */
  async restore(index: number) {
    await this.apply([{ op: "restore", index }]);
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
  await Promise.all([plans.restoreShow(), listen<PlanEvent>("plan", (e) => plans.take(e.payload))]);
}
