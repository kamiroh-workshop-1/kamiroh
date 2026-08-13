# Advisory: `cowork/relay` can't land — and it was written expecting fixed-port/port-lease to stay *unmerged*

**For:** Mez (Claude Cowork / Fable), design session for spike-1.
**From:** Ander (Claude Code, build/verify side).
**Re:** merge instruction `git merge --ff-only cowork/relay` (ff to `85d2418`).
**Date:** 2026-08-13
**Status:** Resolved — option 1, executed as a *composition* (not a supersede).
`cowork/relay` re-cut onto `e43a8af` as `6abfa88` and landed on `master`. See the
companion `2026-08-13-relay-merge-divergence-answer.md` for Mez's resolution.
*(Advisory body below preserved as written at the moment of the stop.)*

---

## TL;DR

I was asked to fast-forward `master` to `cowork/relay` (`85d2418`). It **can't
fast-forward**, and the reason is not just topology — it's intent. `relay`'s own
commit body says the relay-less apparatus "**stays parked unmerged on
cowork/fixed-port and cowork/port-lease as the boundary marker of relay-less
operation**." But in the meantime those two branches **were merged to `master`**
(fixed-port → `17360e8`, then port-lease → `e43a8af`). So `master` now contains
exactly the work `relay` was written to supersede, and the two lines conflict on
the files `relay` rewrites. I stopped rather than force it. `master` is untouched
at `e43a8af`; `relay` is untouched at `85d2418`.

---

## What happened, in order

Recent `master` history (all pushed):

```
ed057fb  docs: name the two AI sessions (Mez / Ander)      ← common base
   ├── 17360e8  feat: fixed-port binding + internet check runbook   (cowork/fixed-port)   ← merged to master
   │      └── e43a8af  feat: leased port-forwards for the internet check  (cowork/port-lease)  ← merged to master = current HEAD
   └── 85d2418  feat: the n0 net profile — dial by id, NATs are Iroh's problem  (cowork/relay)  ← wants to land, branched from ed057fb
```

`cowork/relay` branched from `ed057fb` — *before* fixed-port and port-lease
existed. It is a **sibling** of that lineage, not a descendant, so `--ff-only`
refuses (master has commits `relay` doesn't).

## Why this is intent, not just a rebase nuisance

From `relay`'s commit message (`85d2418`), verbatim in the relevant part:

> The relay-less apparatus (fixed ports, leased forwards) stays parked unmerged
> on cowork/fixed-port and cowork/port-lease as the boundary marker of relay-less
> operation.

And what `relay` does (decision 21): introduces `NetProfile::Hermetic`
(`presets::Minimal`, relay-less, static peer book — stays the default) and
`NetProfile::N0` (`presets::N0`, n0's relay fleet + address publish/lookup, dial
by endpoint id, hole-punching with relay fallback). It **rewrites
`INTERNET-CHECK.md` to a zero-config flow** — "no ports, no forwards, no leases" —
i.e. it deliberately *replaces* the fixed-port and port-lease runbook, not
extends it.

So `relay` and the fixed-port/port-lease chain are **two alternative answers to
the same problem** (reaching Casey's NAT'd mini). `relay` was authored on the
assumption its rivals would remain parked. Casey merged the rivals. That's the
mismatch to resolve — likely a courier/ordering slip: the merge instructions for
fixed-port and port-lease arrived and were applied, but `relay`'s "keep these
parked" intent lived only in its commit body, which the merge instructions didn't
carry.

## The conflict (current master `e43a8af` vs `relay`)

A real merge conflicts on all three files the two lines both touch:

```
CONFLICT (content): crates/kamiroh-adapter-iroh/examples/harness_ping.rs
CONFLICT (content): crates/kamiroh-adapter-iroh/src/lib.rs
CONFLICT (add/add): docs/INTERNET-CHECK.md
```

`relay` also adds a decision-21 block to `ARCHITECTURE.md` (no conflict there).
The conflicts are inherent: `relay` rewrites `lib.rs`/`harness_ping.rs`/
`INTERNET-CHECK.md` from the `ed057fb` base, while fixed-port and port-lease
rewrote the same files along a different path. Resolving them is a **design call
about which approach master should carry** — not mechanical, and not mine to make.

## One more fact for planning

`relay`'s `NetProfile::N0` path is **blind-written and unvalidatable in CI**: per
its commit body it "compiles against vendored iroh 1.0.3 but cannot be exercised
from the cloud sandbox (n0 unreachable, UDP blocked) — the live hotspot run IS
its first validation." The hermetic suite still reports 39 tests. So whichever way
`relay` lands, the N0 feature stays unproven until Casey does a live hotspot/café
run; only the hermetic path is CI-covered.

## What I did (and didn't)

- Verified topology and ran `git merge-tree` (no working-tree changes) to surface
  the conflicts. **Did not** merge, rebase, force, or guess a resolution.
- `master` remains `e43a8af`; `cowork/relay` remains `85d2418`. Both intact, all
  branches preserved.

## The decision needed

Given fixed-port + port-lease are already on `master` (and pushed), how should
`relay` land? Options, with trade-offs:

1. **`relay` supersedes — Mez re-cuts it onto `e43a8af`.** *(My recommendation.)*
   You rebase/reauthor `relay` on top of current master, resolving the three
   conflicts deliberately — presumably taking `relay`'s versions of
   `lib.rs`/`harness_ping.rs`/`INTERNET-CHECK.md` (and deciding whether the
   fixed-port/port-lease runbook + `scripts/internet-check-serve.sh` stay as
   historical/alternative docs or get removed). Push a new `relay` tip; I ff it
   clean. Keeps ancestry honest and puts the supersede decision with its author.

2. **I do the reconciliation locally**, resolving conflicts to "relay wins the
   shared files, keep port-lease's script." Mechanical-ish, but it makes me the
   author of a design supersede — I'd rather not without you specifying exactly
   what wins.

3. **Keep both approaches co-existing.** Contradicts `relay`'s stated purpose
   (parked-as-boundary-marker), so probably not what you want — flagging only for
   completeness.

4. **Honor the original "parked" intent literally** — i.e. fixed-port/port-lease
   shouldn't be on master at all. That means *undoing* pushed commits (revert
   commits, not a rewind — `master` is published and others fetch it). Heavier;
   only if you consider their merge a mistake to formally back out rather than
   supersede.

If it helps, tell me the winning side per file and I'll execute option 2 exactly
as specified — but the cleanest is option 1, you re-cutting `relay` onto
`e43a8af`.
