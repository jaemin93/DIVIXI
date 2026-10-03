# Third-party notices

DIVIXI itself is Apache-2.0 (see [LICENSE](LICENSE)). This file covers the
**dependencies** it is built from: what they are licensed under, which of them
end up inside a release, and which are only fetched on your own machine.

This is a courtesy summary, not a legal instrument. It is also not the same
thing as [NOTICE](NOTICE):

| File | Covers |
|---|---|
| [NOTICE](NOTICE) | Attribution DIVIXI **owes** under Apache-2.0 §4, for code adapted from Kiro Crew |
| **THIRD-PARTY-NOTICES.md** (this file) | The licences of the **dependency tree**, and what is redistributed |

If you only want to know "can I ship this": nothing in the tree is GPL, AGPL,
SSPL, or commercially restricted. The details are below.

---

## 1. What a release actually contains

A DIVIXI installer contains exactly two things built from third-party code:

1. **The Rust binary**, with every Cargo dependency compiled into it.
2. **`dist/`**, the bundled UI, containing the eight runtime npm packages in
   `dependencies`.

`tauri.conf.json` sets no `bundle.resources`, so **`node_modules` is never
shipped.** Everything in `devDependencies` — Vite, the Svelte compiler,
TypeScript, prettier, svelte-check, the ACP adapters — is a build-time or
development-time tool and is not redistributed. See §4, which matters most.

---

## 2. Rust dependencies

632 entries in `Cargo.lock`; 625 are external crates and 7 are DIVIXI's own
workspace members. Licences were read from each crate's own `Cargo.toml`
(`os-audit/scan-cargo-licenses.mjs` reproduces this).

**No GPL, AGPL, LGPL-only, SSPL, BUSL, or commercial licence appears anywhere in
the tree.**

The distribution is overwhelmingly permissive:

| Licence | Crates |
|---|---|
| `MIT OR Apache-2.0` and its spellings | ~370 |
| `MIT` | 149 |
| `Unicode-3.0` | 18 |
| `Zlib OR Apache-2.0 OR MIT` | 18 |
| `Unlicense OR MIT` | 11 |
| `Apache-2.0` | 6 |
| `MPL-2.0` | 5 |
| `BSD-3-Clause`, `BSD-2-Clause`, `ISC`, `Zlib`, `0BSD`, `CC0-1.0`, `BSL-1.0`, `CDLA-Permissive-2.0` | the remainder |

Four groups are worth naming explicitly.

### MPL-2.0 (5 crates)

`cssparser`, `cssparser-macros`, `dtoa-short`, `selectors`, `option-ext`.

MPL-2.0 is **file-level** copyleft. It reaches the MPL-licensed files
themselves, not the program that links them, so it does not affect DIVIXI's own
licence. The obligation it does create: if you distribute a build, the MPL text
must be available to recipients, and any modification you make *to those files*
must be offered under MPL-2.0. DIVIXI does not modify them.

> https://www.mozilla.org/en-US/MPL/2.0/

### Unicode-3.0 (18 crates)

The ICU crates (`icu_*`, `zerovec`, `yoke`, `tinystr`, `writeable`,
`potential_utf`, `zerotrie`, `zerofrom`, `litemap`) plus `unicode-ident`, which
is `(MIT OR Apache-2.0) AND Unicode-3.0`. The Unicode licence is permissive and
requires the notice be kept with the data.

> https://www.unicode.org/license.txt

### CDLA-Permissive-2.0 (1 crate)

`webpki-root-certs` — the Mozilla root certificate set, published as *data*
under a permissive data licence. No copyleft, no attribution burden on binaries.

### Multi-licensed crates offering a copyleft option

`r-efi` 5.3.0 and 6.0.0 are `MIT OR Apache-2.0 OR LGPL-2.1-or-later`. Because
the operator is **OR**, taking MIT or Apache-2.0 discharges the licence
entirely; the LGPL option carries no obligation unless you choose it. (`r-efi`
is a UEFI-target transitive dependency and is not linked into desktop builds.)
`ryu` is `Apache-2.0 OR BSL-1.0` — same reasoning, take Apache-2.0.

---

## 3. npm dependencies

184 packages installed. **No GPL or AGPL.** Counts: MIT 150, Apache-2.0 11,
ISC 8, BSD-3-Clause 5, `Apache-2.0 OR MIT` 3, MPL-2.0 2, BSD-2-Clause 1,
Unlicense 1, and the two Anthropic packages covered in §4.

### Shipped in `dist/` — the nine runtime dependencies

These are bundled into the UI and therefore redistributed:

| Package | Version | Licence |
|---|---|---|
| `@tauri-apps/api` | 2.11.1 | `Apache-2.0 OR MIT` |
| `@xterm/xterm` | 6.0.0 | MIT |
| `@xterm/addon-fit` | 0.11.0 | MIT |
| `dompurify` | 3.4.15 | `MPL-2.0 OR Apache-2.0` |
| `highlight.js` | 11.12.0 | BSD-3-Clause |
| `marked` | 18.0.14 | MIT |
| `marked-highlight` | 2.2.4 | MIT |
| `perfect-freehand` | 1.2.3 | MIT |
| `qrcode-generator` | 2.0.4 | MIT |

`qrcode-generator` is Kazuhiko Arase's reference QR encoder and has no
dependencies of its own. It draws the pairing code for phone access; the QR
specification is DENSO WAVE's, and "QR Code" is their registered trademark,
which costs nothing to use but is noted because the package's own header
does.

`dompurify` is dual-licensed with an **OR**, so DIVIXI takes the Apache-2.0
option and no MPL obligation arises for it. `highlight.js` (BSD-3-Clause) and
the MIT packages require their copyright notice be preserved in distributions;
`dist/assets/*.js` retains the bundled license comments.

### Build-time only, not redistributed

`lightningcss` and `lightningcss-win32-x64-msvc` are MPL-2.0. They are Vite's
CSS transformer, run at build time. They are not in `dependencies`, are not
bundled, and never reach a user's machine.

---

## 4. The ACP adapters and the Anthropic SDK — fetched, not redistributed

This is the part a reviewer is most likely to ask about, so it is spelled out.

DIVIXI drives Claude Code and Codex over the Agent Client Protocol through two
npm adapters:

- `@agentclientprotocol/claude-agent-acp`, which depends on
  **`@anthropic-ai/claude-agent-sdk`** and its platform binary packages
  (`@anthropic-ai/claude-agent-sdk-<platform>`, each containing a prebuilt
  `claude` executable);
- `@agentclientprotocol/codex-acp`.

Both Anthropic packages are **proprietary**. Their `LICENSE.md` reads, in full:

> © Anthropic PBC. All rights reserved. Use is subject to the Legal Agreements
> outlined here: https://code.claude.com/docs/en/legal-and-compliance.

**DIVIXI does not redistribute them.** Two facts establish that:

1. `tauri.conf.json` declares no `bundle.resources`, so an installer contains
   only the Rust binary and `dist/`. No `node_modules` is packaged.
2. At runtime, `install_npm_adapter()` in `crates/agents/src/lib.rs` runs
   `npm install --prefix <data>/adapters <package>` **on the user's own
   machine**, fetching from the public npm registry. Until that finishes it
   falls back to `npx`, which also fetches locally. A development build instead
   uses the repository's own `node_modules`.

So the adapters and the Anthropic SDK are acquired by the user's npm, under
Anthropic's own terms, exactly as if the user had typed the install command.
DIVIXI is the thing that calls npm, not a redistributor. They appear in
`devDependencies` so that a development checkout can run an agent without the
install step.

The same holds for the agent CLIs themselves — Claude Code, Codex, GitHub
Copilot CLI, Antigravity. DIVIXI locates and launches whatever the user has
already installed and signed into; it neither bundles nor relicenses them.
Antigravity's ACP server is downloaded from Google at the user's request.

> **If this ever changes, revisit this section.** Setting `bundle.resources` to
> ship `node_modules` would turn the above into redistribution of a proprietary
> binary, and Anthropic's Legal Agreements would have to be read properly
> first. The terms are not vendored with the package — they live at the URL
> above — so they can also change without a version bump.

---

## 5. Reproducing this

The two scanners used here read licences out of the installed trees rather than
trusting a summary:

```bash
node os-audit/scan-cargo-licenses.mjs   # Cargo.lock against ~/.cargo/registry
node os-audit/scan-npm-licenses.mjs     # node_modules
```

Neither is wired into CI. For a machine-checked gate, `cargo-deny` (Rust) with
a `deny.toml` licence allowlist is the usual choice, and would catch a
copyleft dependency arriving through a transitive bump.
