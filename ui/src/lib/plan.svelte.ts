import { invoke, listen, local } from "./ipc.svelte";
import { keepsSetting } from "./layoutSettings";
import { DRAWER_HEIGHT_KEY, DRAWER_KEY, DRAWER_RESET, clampDrawer, drawerFromSettings, pickStep } from "./planDrawer";
import { store, artifactKey, type ArtifactInfo, type OptionConfig, type Routine } from "./store.svelte";
import { t } from "./i18n.svelte";
import { emptyPlan, instructionChanged, needsFirstDraft, pressedRun, runGate, unsaved, type LiveRun, type PlanDoc, type PlanEvent, type RunGate } from "./planGraph";

/** Mirrors `routine::StepProgress`: a run's agent saying where it is (status "begun": a run given a plan started). */
type StepProgress = { routine: string; run: string; step: string; status: string; note: string };

/**
 * Plans whose first draft was asked for, for as long as the app runs: a
 * second press, or coming back to the page while the agent works, does not
 * ask again. (Across a restart the routine already has its plan, which
 * asks nothing either.)
 */
const drafted = new Set<string>();

/**
 * A routine's plan on screen (src-tauri/src/plan.rs keeps it): its steps
 * as a graph, the step picked, its saved versions, and the moves that
 * change it. The conversation with the plan's agent is the store's artifact
 * conversation (store.planInfo, opened from the robot toggle), as a
 * design's is; this is the plan beside it.
 */

/** What `routine_plan_open` answers. */
type Opened = { routine: Routine; plan: ArtifactInfo; doc: PlanDoc };

/** An agent and its options, as picked. */
export type AgentChoice = { agent: string; config: OptionConfig };

class Plans {
  /** The routine whose plan this is, and whether its plan is on screen. */
  routine = $state("");
  open = $state(false);
  doc = $state<PlanDoc>(emptyPlan());
  /** The plan's artifact id, once known. */
  id = $state("");
  /** The step picked: the drawer shows it when the drawer is open. */
  picked = $state("");
  /** The step drawer under the graph, toggled as the terminal is; kept. */
  drawerOpen = $state(false);
  drawerHeight = $state(DRAWER_RESET);
  busy = $state(false);
  /** The run whose step results the plan shows; empty is the newest that has any. */
  resultRun = $state("");
  /** Bumped when the graph should frame the whole plan again (after a layout). */
  refit = $state(0);
  /** The routine's run in flight, as its plan shows it; null when not running. */
  live = $state<LiveRun | null>(null);
  /** Run was pressed and the plan on screen is not what the run would get: what to ask. */
  gate = $state<RunGate | null>(null);
  /**
   * The planning agent a new plan starts on, picked beside [Plan] before the
   * plan exists. Once it does, the plan's own agent (the panel's picker) is it.
   */
  draftAgent = $state<AgentChoice>({ agent: "", config: {} });

  /** Start the picker beside [Plan]: the planning agent last used, else `fallback`. */
  async loadDraftAgent(fallback: string) {
    let got: AgentChoice | null = null;
    try {
      got = await invoke<AgentChoice | null>("plan_agent_default");
    } catch {
      // No memory yet reads as none.
    }
    this.draftAgent = got ?? { agent: fallback, config: {} };
  }

  /**
   * Run a routine, asking first when its plan on screen is not what the
   * run would get (never saved, or changed since the last save). A routine
   * with no plan runs straight away, as before.
   */
  async requestRun(routine: string) {
    this.gate = null;
    if (store.routine !== routine) await store.openRoutine(routine);
    const r = store.routines.find((x) => x.id === routine);
    if (r?.plan && !(this.open && this.routine === routine)) await this.openFor(routine);
    if (!r?.plan || this.routine !== routine || !this.id) {
      await store.runRoutine(routine);
      return;
    }
    const g = runGate(this.doc);
    if (g === "run") await store.runRoutine(routine);
    else this.gate = g;
  }

  /** Save the working plan as a new version, then run with it. */
  async saveAndRun() {
    const routine = this.routine;
    if (!(await this.save(""))) return;
    this.gate = null;
    await store.runRoutine(routine);
  }

  /** Run with the last saved version, leaving the working plan as it is. */
  async runLastSaved() {
    this.gate = null;
    await store.runRoutine(this.routine);
  }

  /** Read the run in flight (opening the plan, or a run beginning). */
  async loadLive() {
    const routine = this.routine;
    if (!routine) return;
    try {
      const got = await invoke<LiveRun | null>("routine_live", { id: routine });
      if (this.routine !== routine) return;
      // The core registers a run once its agent is open, a moment after Run
      // was pressed: until then, the run shown at the press stands.
      if (!got && this.live && !this.live.run && store.routineLive(routine)) return;
      this.live = got;
    } catch {
      // Cosmetic: the run's result still comes at its end.
    }
  }

  /** Run was pressed: show the run at once, as the last saved version has it. */
  started(routine: string) {
    if (routine !== this.routine || !this.open) return;
    this.resultRun = "";
    this.live = pressedRun(this.doc);
  }

  /** A run's agent said where it is in the plan. */
  progress(e: StepProgress) {
    if (e.routine !== this.routine) return;
    if (e.status === "begun" || !this.live || (this.live.run && this.live.run !== e.run)) {
      void this.loadLive();
      return;
    }
    const status = e.status as LiveRun["progress"][string]["status"];
    this.live = { ...this.live, run: e.run, progress: { ...this.live.progress, [e.step]: { status, note: e.note } } };
    // The finished run's results are what the cards show next; read them then.
  }

  /** A run ended: the plan shows its results from here. */
  ended(routine: string) {
    if (routine !== this.routine) return;
    this.live = null;
    this.resultRun = "";
  }

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
    this.live = null;
    this.gate = null;
    store.planInfo = null;
    store.planSelected = [];
  }

  /** The plan's agent has a turn in flight, or one waiting in line. */
  get agentBusy(): boolean {
    if (!this.id) return false;
    const key = artifactKey(this.id);
    return store.busyFor(key) || store.queued.some((q) => q.key === key && !q.held);
  }

  /** The first draft is being made: asked for, and the agent still at it. */
  get drafting(): boolean {
    return !!this.id && drafted.has(this.id) && this.agentBusy;
  }

  /**
   * Open the routine's plan, made when it has none. The first time (the
   * routine had no plan), the planning agent is asked for a draft from the
   * instruction: the request and the reply are in its panel's history, the
   * panel stays as it was (it does not open by itself), and the graph shows
   * the draft arriving as suggestion cards. A routine that has a plan just
   * shows it; changing it is asked for in the panel.
   */
  async openFor(routine: string, agent?: AgentChoice): Promise<boolean> {
    const hadPlan = !!store.routines.find((r) => r.id === routine)?.plan;
    this.follow(routine);
    this.busy = true;
    try {
      // A new plan is made on the planning agent picked beside [Plan].
      const pick = !hadPlan && agent?.agent ? agent : null;
      const got = await invoke<Opened>("routine_plan_open", { routine, agent: pick?.agent ?? null, config: pick?.config ?? null });
      if (this.routine !== routine) return false;
      this.id = got.plan.id;
      this.doc = got.doc;
      this.open = true;
      store.planInfo = got.plan;
      void this.loadLive();
      // The routine now carries its plan's id; the list hears it too.
      store.routines = store.routines.map((r) => (r.id === routine ? got.routine : r));
      void store.refreshArtifactSession();
      if (needsFirstDraft({ hadPlan, doc: got.doc, asked: drafted.has(got.plan.id), busy: this.agentBusy })) {
        drafted.add(got.plan.id);
        void store.sendToArtifact(got.plan.id, got.plan.agent, t("plan.ask.draft"));
      }
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

  /**
   * Save the working plan: as a new version, or over version `over`. The
   * plan is read back before this returns, so what comes next (a run shown
   * at once) sees the saved version.
   */
  async save(note: string, over?: number): Promise<boolean> {
    const out = await this.apply([over ? { op: "save", note, over } : { op: "save", note }]);
    if (out[0]?.ok !== true) return false;
    try {
      const doc = await invoke<PlanDoc>("plan_doc", { id: this.id });
      if (doc.version >= this.doc.version) this.doc = doc;
    } catch {
      // The event brings it anyway.
    }
    return true;
  }

  /** Bring a saved version back into the working plan. */
  async openVersion(n: number) {
    await this.apply([{ op: "open", n }]);
    setTimeout(() => this.refit++, 60);
  }

  /** Pick a step: it goes with the next message, and an open drawer shows it. It does not open the drawer. */
  pick(id: string, add = false) {
    const next = pickStep({ selected: store.planSelected, picked: this.picked, open: this.drawerOpen }, id, add);
    store.planSelected = next.selected;
    this.picked = next.picked;
  }

  /** Raise or lower the step drawer; kept, as the terminal is. */
  setDrawer(open: boolean) {
    this.drawerOpen = open;
    keep(DRAWER_KEY, open ? "open" : "closed");
  }

  setDrawerHeight(px: number, persist = false) {
    this.drawerHeight = clampDrawer(px, window.innerHeight);
    if (persist) keep(DRAWER_HEIGHT_KEY, String(this.drawerHeight));
  }

  /** The drawer as it was left (this PC's window; a phone's lasts the page). */
  async restoreDrawer() {
    if (!keepsSetting(DRAWER_KEY, local)) return;
    const read = (key: string) => invoke<string | null>("get_setting", { key }).catch(() => null);
    const [open, height] = await Promise.all([read(DRAWER_KEY), read(DRAWER_HEIGHT_KEY)]);
    const d = drawerFromSettings(open, height);
    this.drawerOpen = d.open;
    this.drawerHeight = clampDrawer(d.height, window.innerHeight);
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

/** Write a layout setting this window keeps. */
function keep(key: string, value: string) {
  if (!keepsSetting(key, local)) return;
  invoke("set_setting", { key, value }).catch((err) => (store.lastError = String(err)));
}

export const plans = new Plans();
store.routineStarted.push((id) => plans.started(id));

/** Subscribe once. */
export async function connectPlans() {
  void plans.restoreDrawer();
  await Promise.all([
    listen<PlanEvent>("plan", (e) => plans.take(e.payload)),
    listen<StepProgress>("routine_progress", (e) => plans.progress(e.payload)),
    listen<{ routine: string }>("routine_done", (e) => plans.ended(e.payload.routine)),
  ]);
}
