<!-- Thanks for the pull request. Fill in what applies and delete what does not —
     a one-line fix does not need every heading below. CONTRIBUTING.md has the
     details: https://github.com/jaemin93/divixi/blob/main/CONTRIBUTING.md -->

## What this does

<!-- One or two sentences, in user terms: what is different after this lands. -->

## Why

<!-- For a fix: the symptom, and what was actually causing it.
     For a feature: what you could not do before, and why this approach over the
     alternatives you considered.
     Link the issue if there is one: Fixes #123 -->

## What I did not do

<!-- Deliberately out of scope, so a reviewer does not ask for it. Delete if
     nothing applies. -->

## Checks

<!-- Tick what you ran, on your own machine. "Did not run, because …" is a more
     useful answer than a tick you are not sure about — say so rather than
     leaving it blank. An agent reporting a green run is not a green run:
     https://github.com/jaemin93/divixi/blob/main/CONTRIBUTING.md#working-with-ai-tools -->

- [ ] `npm run check` — UI types
- [ ] `npm test` — UI unit tests
- [ ] `npm run check:i18n` — ko and en hold the same keys
- [ ] `npm run check:readme` — the two READMEs still have the same shape
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets` — no new warnings
- [ ] `npm run build` (needed before any cargo command on a fresh clone)

Notes on the above:

<!-- Anything that failed, was skipped, or only ran on one platform. -->

## Tests

<!-- What you added or updated, and the behaviour each one locks in. If the change
     is not testable in a unit test, say what you did by hand instead and on which
     platform. -->

## If this touches the UI

- [ ] Every new string is in **both** `ko` and `en` in `ui/src/lib/i18n.svelte.ts`
- [ ] Screenshot or recording attached below for anything user-visible

<!-- Attach media to the pull request; do not commit it. -->

## If this touches anything else on the list

- [ ] New top-level directory? A `!` line for it is in `.gitignore` in this same
      commit — otherwise git does not see the files
- [ ] New dependency, or code/prompt/data from another project? Recorded in
      `NOTICE` or `THIRD-PARTY-NOTICES.md`, and its origin named here
- [ ] Changed documented behaviour? The doc is updated in this same commit
- [ ] Nothing new that holds a secret derives `Debug`, and nothing secret reaches
      the log, the store, `crash.log` or the diagnostics report

<!-- Please do not reformat files this change does not otherwise touch:
     `cargo fmt` and prettier are deliberately not gated, and a reformat has to
     land on its own so `git blame` stays useful. CONTRIBUTING.md explains why. -->
