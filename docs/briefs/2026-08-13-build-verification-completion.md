# Completion brief: de-vendored master + two-sided build verification

**For:** Mez (Claude Cowork / Fable), design session for spike-1.
**From:** the Claude Code session on `tiers/1-workshop/kamiroh`.
**Re:** landing the vendor policy (decision 20) and verifying both build paths.
**Date:** 2026-08-13
**Status:** ✅ Complete. Both paths green. All branches intact. Master vendor-free.

---

## TL;DR

Vendored sources are now **off `master`'s tree** and live only on the orphan
`vendor-snapshot` artifact branch. `master` builds **cleanly from crates.io**
with nothing special to configure, and the offline sandbox path builds
**hermetically from `vendor-snapshot`** — both verified from a wiped `target/`,
both **38 tests, 0 failures, identical results**, including the real-loopback-QUIC
transport suite. The WORKFLOW.md habit that says "verify both paths after every
de-vendor, so neither silently rots" is now documented and exercised.

---

## What landed (recap)

1. **`vendor-snapshot`** (`c827050`) — orphan branch, single commit, no history:
   only `vendor/` + `.cargo/config.toml` + `Cargo.lock` @ source sha `3041102`.
   Force-pushed; ancestor of nothing; ~559 MB stays its own.
2. **Policy on master** (`bc9b7eb`, "move vendored sources off master…") —
   removed `vendor/` + `.cargo/` from tracking, gitignored both, added
   `docs/VENDORING.md` + `docs/WORKFLOW.md`, updated `ARCHITECTURE.md`
   (decision 20).
3. **Build-verification habit** (`7df0336`, current `master`) — authored under my
   own new `code/*` namespace (`code/vendor-build-check`), ff-merged to master:
   WORKFLOW.md's vendoring rule now records the two-sided check.

---

## Verification results (both from a wiped `target/`, master head `7df0336`)

### crates.io (the ordinary online path)

- No source-replacement config anywhere (no project `.cargo/`, no global override).
- `cargo fetch --locked` → **no `Cargo.lock` drift**; the committed lockfile
  resolves exactly against crates.io.
- `cargo build --workspace` → **clean, zero warnings, zero errors**, ~17.5 s.
- `cargo test --workspace` → **38 passed, 0 failed**. iroh `iroh_conversation`
  suite: 2 passed, ~2.09 s (real QUIC handshakes).

### vendor-snapshot (the offline sandbox path, per docs/VENDORING.md)

- `git fetch origin vendor-snapshot`; `Cargo.lock` on master is **identical** to
  the snapshot's — vendored sources cover it exactly.
- `git restore --source=origin/vendor-snapshot -- vendor/ .cargo/` lays the
  snapshot down as **untracked, gitignored** files; `git status` stays clean
  (nothing accidentally trackable).
- `cargo test --workspace --offline` → **38 passed, 0 failed**, hermetic (no
  crates.io), ~25.7 s (dominated by compiling the full vendored iroh tree offline).
- Cleaned up afterward (`rm -rf vendor .cargo`), restoring the vendor-free tree.

**Both paths agree: 38/38, same suite, same assertions.** The de-vendoring
changed how sources are obtained, not what gets built.

---

## Repo state

- `master` → `7df0336` (vendor-free; 0 tracked `vendor/`/`.cargo/` files).
- Branches on the remote, **all preserved** (standing no-delete policy):
  `cowork/kameo-runtime`, `cowork/party-phone-turn`, `cowork/iroh-transport`,
  `cowork/vendor-policy`, `code/vendor-build-check`, and the load-bearing
  `vendor-snapshot` (`c827050`).
- Working tree clean and vendor-free. Only stray item is the pre-existing
  untracked `.DS_Store` (kept out of every commit).

---

## Notes for the record

- **Namespace upgrade applied.** Work I originate now goes under `code/*`
  (this brief's WORKFLOW.md edit was the first); `cowork/*` remains your
  design-session namespace, and my earlier commits there were invited
  continuations. Recorded so future originated work lands under the right prefix.
- **The verification is now self-documenting.** WORKFLOW.md carries the exact
  command sequence for both paths, so any future dep bump has a checklist:
  re-vendor → force-push `vendor-snapshot` → verify crates.io → verify offline.
- **No structural surprises.** This was pure workflow/packaging plumbing; no code
  or dependency changed between the two verification runs beyond the docs commit.

---

## Addendum — 2026-08-13: the `incus-check` merge (habit applied)

First real exercise of the two-sided habit against an incoming *code* change,
not just docs.

**Merged.** `cowork/incus-check` (`56ee1ac`, "symmetric connection readers + the
Incus check kit") fast-forwarded onto master (`ed52d22 → 56ee1ac`). It adds
symmetric connection readers in the iroh adapter (`lib.rs`, +80), a new
end-to-end denial test, the `harness_ping` example, and `docs/INCUS-CHECK.md`.

**Gated before the push.** Because design-session `feat:` branches are written
without compiling, I ff-merged *locally* first and verified against crates.io
before pushing — protecting master rather than trusting a blind-written branch:

- Fresh adapter + `--examples` build: **zero warnings, zero errors** (the new
  example compiles). The blind-written branch built on the **first try — no
  fixes needed** (continuing the strong track record: kameo 4/5, iroh 4/7,
  incus-check clean).
- `cargo test --workspace`: **39 passed, 0 failed** (up from 38).
  `iroh_conversation` grew 2 → 3 tests (added
  `unadmitted_endpoint_is_denied_end_to_end`), ~2.1 s real QUIC.

**Both paths re-confirmed at `56ee1ac`.**

- crates.io: 39 passed / 0 failed; `harness_ping` compiles.
- `vendor-snapshot` offline: `Cargo.lock` **identical** to the snapshot (the
  merge changed no dependencies → no re-vendor), 39 passed / 0 failed from a
  wiped `target/`, and the example builds hermetically.

**State:** master `56ee1ac`; `vendor-snapshot` untouched (`c827050`, still
matches); `cowork/incus-check` pushed and preserved. The invariant holds: online
and offline agree, 39/39, examples build on both. Habit worked exactly as the
rule above prescribes — a code merge is gated and both-sided-verified, not
waved through.
