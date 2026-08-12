# Completion brief: kameo 0.22 adapter port — done, pushed, awaiting review

**For:** Mez (Claude Cowork / Fable), design session for spike-1.
**From:** the Claude Code session on `cowork/kameo-runtime`.
**Re:** follow-up to `kameo-ports-send-advisory.md` / `-answer.md`.
**Date:** 2026-08-12
**Status:** ✅ Complete. Branch pushed. `master` untouched. No PR opened (left for review).

---

## TL;DR

Your Option-A-in-the-spirit-of-C direction was executed exactly as written. The
`+ Send` port change, the retirement of `#![allow(async_fn_in_trait)]`, and
ARCHITECTURE.md decision 15 are all in. Three commits land in the order you
specified; `cargo test --workspace --offline` is green at **30 tests**; the
branch is pushed. The blind-written adapter's API guesses held up on **4 of 5**
points — only `spawn` had drifted.

---

## The 5 flagged API points (kameo 0.17 → real 0.22.2)

| # | Flagged point | Changed? | Finding |
|---|---|---|---|
| 1 | `Actor` trait shape (`Args`/`Error`/`on_start`) | No | Matches exactly. `Infallible` satisfies the new `ReplyError` bound via a blanket impl. |
| 2 | `kameo::spawn(actor)` | **Yes** | Free function gone; now the `Spawn` trait method `Host::spawn(host)` → `ActorRef` directly. |
| 3 | `tell(msg).await` | No | `TellRequest: IntoFuture` → `Result<(), SendError<M>>`; existing `.is_err()` fine. |
| 4 | `Context` path / lifetimes | No | `kameo::message::Context<Self, Self::Reply>` unchanged. |
| 5 | `kill` / `stop_gracefully` | No | `ActorRef::kill()` exists exactly as used. |

The 4-of-5 confirmation is now recorded in the `fix(adapter-kameo)` commit body,
as you noted it would calibrate how the Iroh adapter gets written.

---

## What shipped

**Authorized ports changes (decision 15):**
- `Transport::send` and `Inbox::next`: `async fn` → `fn -> impl Future<…> + Send`.
- Crate-level `#![allow(async_fn_in_trait)]` "spike scope" line removed.
- One-line `Send`/decision-15 pointer appended to each method's doc comment.
- `ARCHITECTURE.md`: decision 15 appended verbatim per your answer file.
- Backward-compatible: memory adapter's `async fn` impls and the single-threaded
  `LocalRuntime` needed no edits.

**Adapter changes:**
- `kameo = "0.17"` → `"0.22"`.
- `use kameo::actor::{ActorRef, Spawn};`; `kameo::spawn(host)` → `Host::spawn(host)`.
- `+ Sync` added to the `T` transport bound across all where-clauses (kameo needs
  `Actor: Send` + `Args: Send`; `Host` holds `Arc<Inner<T,R>>`, so `Inner: Sync`,
  so `T: Sync`). Roster/pump/admission structure untouched.

**Vendoring:**
- `cargo vendor vendor` + `.cargo/config.toml` (crates-io → `vendored-sources`).

---

## Commits (in the order you specified), on `cowork/kameo-runtime`

```
2dd5442 chore(vendor): vendor dependencies for offline builds
f79f1bb fix(adapter-kameo): adapt to kameo 0.22
3099a77 refactor(ports): require Send futures on Transport::send and Inbox::next
78a989d feat: Kameo runtime adapter (blind-written; first build happens locally)  <- prior HEAD
```

The `refactor(ports)` commit body cites the advisory exchange as the authorization
for lifting the "adapter crate only" constraint, and attributes the change to
decision 15 rather than burying it in a `fix:`.

---

## Verification

- `cargo test --workspace --offline` from a wiped `target/`: **30 passed, 0 failed.**
  - `kameo_conversation.rs`: **2** (both, as written — no assertions weakened)
  - `harness_conversation`: 2 · `request_ack`: 2 · adapter-memory: 5 · app: 10 · domain: 9
- `vendor/`: **28 MB**, 31 top-level crates (1,767 files).

---

## State + open items for the humans/Mez

- Branch **pushed**: `origin/cowork/kameo-runtime`. `origin/master` still at `f415f5c` — **no merge**.
- **No PR opened.** GitHub printed its usual "create a PR" suggestion on push; deliberately
  not acted on, since the instruction was push-only with review first. Opening the PR is
  yours/Mez's call whenever ready.
- Housekeeping: a stray untracked `.DS_Store` sits in the repo root (pre-existing, not from
  this work). Kept out of every commit. `.gitignore` currently only ignores `/target` — you
  may want to add `.DS_Store` in a future tidy-up, but I left it alone to stay in scope.
- The two advisory files (`kameo-ports-send-advisory.md`, `-answer.md`) and this brief live
  in `tiers/1-workshop/`, one level above the git repo — intentionally outside version
  control. If you want the advisory exchange preserved in-repo as a decision record, that
  would be a deliberate follow-up.
