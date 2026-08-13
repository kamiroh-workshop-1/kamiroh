# How kamiroh is built

*A human-facing companion to the operational notes in `TIERS.md` (kept
outside this repo) — for future developers curious about, or wanting to
borrow, the workflow behind this project. Written August 2026, mid-spike-1.*

kamiroh is developed by a small ensemble: one human (Casey) making the
decisions and holding the keys, and two named AI sessions doing the drafting,
building, and reviewing — a cloud-based design session with long-lived
context, and local Claude Code sessions with network access and build tools.
The workflow below is what lets that ensemble move fast without stepping on
itself.

## Tiers: one fork per architectural experiment

Development proceeds in **spikes** across a set of repos called **tiers**,
each a fork of the main repo (one org per fork, since GitHub allows one fork
per org):

- **main** (`casey-bowman/kamiroh`) — the canonical repo.
- **staging** (`kamiroh-staging/kamiroh`) — the integration tier. Every
  architectural spike is recorded here as a branch before anything reaches
  main.
- **workshop-N** (`kamiroh-workshop[-N]/kamiroh`) — one fork per
  *architectural spike*: a grand-scale experiment where the fork *is* the
  experiment. Smaller *implementation spikes* are just branches inside a
  workshop fork. This repo is workshop-1, the second architectural spike,
  designed from scratch and deliberately unpolluted by its predecessor's
  choices.

The flow: work lands on a workshop's `master`; when a spike is ready it is
published to staging as a `spike-<name>` branch; staging's mainline
eventually graduates to main.

## The division of labor

The **design session** (cloud), called **Mez**, holds the architecture: it
writes `ARCHITECTURE.md` and most of the code, keeps the glossary and decision
log, and reviews everything before it merges. Its sandbox cannot reach crates.io
or push to GitHub — constraints that shaped two habits worth naming:

- **Blind-writing with assumption lists.** Adapter code against external
  crates (kameo, iroh) is written without compiling, with the API points
  most likely to be wrong explicitly flagged in a doc comment. Track record
  so far: 4/5 correct on kameo, 4/7 on iroh — drift concentrates at
  entry-point/builder surfaces while internals hold, which calibrates each
  next round.
- **The bundle relay.** Finished branches travel as git bundles to the
  human's local clone; the human (or a Code session) pushes. Verification
  closes the loop by anonymous fetch from GitHub.

**Build sessions** (local Claude Code), called **Ander**, do what the cloud
cannot: resolve dependencies, run the first real build, fix mechanical API
drift — under strict scope instructions, with a standing order to **STOP and
write a brief** rather than redesign when something structural surfaces. One such
stop produced the ports-`Send` advisory exchange preserved in
`docs/advisories/` — the full deliberation behind decision 15, kept because
a decision log entry says *what* and an advisory says *why it was hard*.

The **human** merges, pushes, arbitrates advisories, and makes every
decision that outlives the session that raised it.

## The back-and-forth: shuttling between Cowork and Code

The human is the courier between the two AI surfaces, and a few habits make
that shuttle nearly frictionless:

- **Paste-ready handoffs.** The design session ends every work stretch with
  a self-contained instruction block — repo path, branch, context, scope
  limits, the STOP condition, and what to report back. The human pastes it
  into Claude Code verbatim and types nothing else. All context a build
  session needs travels *inside the block*, because Code sessions start
  cold; the accumulated design context stays in the long-lived Cowork
  session, which is treated as the project's working memory.
- **"done" is a complete report.** Coming back the other way, a single word
  suffices when the errand was mechanical — the design session verifies
  results itself by fetching from GitHub rather than trusting a summary.
- **Files, not prose, for anything substantial.** When a build session has
  real findings (a completion brief, an advisory, a question), it writes a
  markdown file *next to* the repo — self-contained, readable without the
  codebase — and the human just says where it is. Answers travel back the
  same way. The best of these exchanges graduate into `docs/advisories/`.
- **Scope discipline makes the shuttle safe.** Build sessions get explicit
  do-not-touch boundaries and a pre-authorized STOP; the design session
  reviews every diff against what it authorized before the human merges.
  The human never has to arbitrate mid-errand — only at the deliberate
  pause points the workflow creates.

## Rules that keep it sane

- **Agent branch namespaces.** Each AI works only under its slash-prefixed
  branches (`cowork/*` for the design session, Mez; `code/*` for the build
  session, Ander; other tools get their own prefixes). Nobody commits to
  `master` directly — it advances only by deliberate merges, done by the human
  or on explicit request.
- **Ancestry is sacred between tiers — with one carved-out boundary.**
  Traffic between long-lived tier mainlines uses plain merge commits or
  fast-forwards, never squash/rebase merges (which re-apply commits under
  new hashes and poison every later ahead/behind comparison). The one
  exception is deliberate: workshop → staging publication is a *content*
  boundary — the spike graduates as a fresh snapshot branch, leaving the
  workshop's heavyweight private history (see below) behind. Within staging
  and staging → main, the ancestry rule applies in full.
- **Vendored sources never touch mainline.** The cloud session needs
  `vendor/` for hermetic offline builds, but committed blobs ride
  ancestry-preserving merges forever — so vendor lives on a force-pushed
  orphan artifact branch (`vendor-snapshot`), and `master` gitignores it.
  `docs/VENDORING.md` has the mechanics. Both build paths are verified after
  every de-vendor, so neither silently rots: `master` must build *the
  ordinary way* against crates.io — wipe `target/`, then
  `cargo fetch --locked && cargo build --workspace && cargo test --workspace`,
  with the committed `Cargo.lock` resolving unchanged and nothing special to
  configure — and the offline path is checked the mirror way, restoring
  `vendor-snapshot` and rerunning under `--offline`. Last confirmed green on
  `master` after the iroh 1.0 bump: identical results both ways (38 tests,
  including the real-loopback-QUIC transport suite).
- **Decisions are written down twice.** The compressed *what* goes in
  `ARCHITECTURE.md`'s numbered decision log; contested decisions keep their
  full deliberation as advisory documents in `docs/advisories/`.

## Toolchains

The two sessions run different Rust toolchains, on purpose and by constraint:

- **Mez's cloud sandbox is pinned by its environment** — currently `rustc
  1.95.0`, and it *cannot change*: the network is allowlisted, so `rustup`
  can't download another toolchain. This is the **floor for language
  features** — code must compile on the sandbox's version.
- **Ander's local toolchain floats newer** — currently `rustc 1.97` — so it
  builds against crates.io and runs the real first build.

We deliberately do **not** pin a `rust-toolchain.toml`: a newer pin would break
Mez's sandbox outright, and pinning the older one would silence exactly the
signal we want. Ander's newer clippy catching lints Mez's older one misses is a
feature, not a discrepancy — those findings arrive via Ander and get triaged
case by case (e.g. the `Waker::noop()` cleanup that surfaced with the relay
landing). If the sandbox's pinned version ever changes, update this note.

## Where to read next

`ARCHITECTURE.md` for the system itself (start with the glossary — the
ubiquitous language is the architecture); `docs/EMBEDDING.md` for using
kamiroh from your own application; `docs/VENDORING.md` for the dependency
scheme; `docs/advisories/` for the hard calls and how they were made.
