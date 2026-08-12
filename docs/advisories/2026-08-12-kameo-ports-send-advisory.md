# Advisory brief: kameo 0.22 adapter blocked by a ports-crate `Send` gap

**For:** an advisor (Claude Cowork / Fable) helping decide how to proceed.
**Prepared by:** the Claude Code session doing the port to kameo 0.22.
**Date:** 2026-08-12
**Repo:** `kamiroh` (Rust workspace, hexagonal architecture), branch `cowork/kameo-runtime` at commit `78a989d`.

---

## TL;DR

A blind-written Kameo actor-runtime adapter (`crates/kamiroh-adapter-kameo`) was bumped from an assumed `kameo 0.17` to the real latest `kameo 0.22.2`. Of the **5 API points the author flagged as likely-wrong, only 1 actually needed changing** (the `spawn` call). Everything compiles *except* for one thing: kameo 0.22 requires actor message-handling futures to be **`Send`**, but the two driven-port trait methods in a **different crate** (`kamiroh-ports`) are bare `async fn` in traits, whose futures carry no `Send` bound.

There is **no stable, adapter-crate-only way** to recover `Send`-ness from an `async fn`-in-trait method. The clean fix is a **2-line signature change in `kamiroh-ports`** (`async fn` → `fn … -> impl Future<…> + Send`), which I verified makes the **entire workspace green (30 tests, including the two `kameo_conversation` tests as written)**.

The problem: the task's operating constraints were **"fix the adapter crate ONLY," "do NOT touch other crates' code,"** and **"if the real API forces a structural change, STOP and report."** The only viable fix crosses a crate boundary the task told me not to cross. Hence this brief — a judgment call is needed.

---

## The task, verbatim constraints

The instruction that governs the work:

> Update the kameo dependency to the latest release, then fix compile errors **IN THAT CRATE ONLY** until `cargo test --workspace` is fully green — including `tests/kameo_conversation.rs`, which must pass **as written**. Keep fixes **minimal and mechanical**: adapt to kameo's real API, but do **NOT** redesign the roster/pump/admission structure, do **NOT** touch other crates' code, and do **NOT** weaken any test assertions. **If kameo's actual API forces a structural change, STOP and report back instead of redesigning.**

Subsequent steps (commit with `fix:` prefix, `cargo vendor`, offline verify, push branch only) are all gated on getting a green build first.

---

## Architecture context (what the advisor needs to know)

`kamiroh` is a hexagon. Relevant crates:

- **`kamiroh-domain`** — pure value types (addresses, vocabulary, allowlist). No async.
- **`kamiroh-ports`** — the trait "ports." Driven ports are implemented by adapters; driving ports are consumed by apps. Depends only on `kamiroh-domain`.
- **`kamiroh-app`** — application layer. Contains the reference `LocalRuntime` (a single-threaded, manually-`step()`ed toy runtime) plus admission/inbound/conversation logic.
- **`kamiroh-adapter-memory`** — in-process transport/registry for tests. Concrete types built on `Arc<Mutex<…>>` (so their futures *are* `Send`).
- **`kamiroh-adapter-iroh`** — a 9-line stub (`//! Will implement …`). No code yet.
- **`kamiroh-adapter-kameo`** — **the crate under work.** A *driving* adapter: hosts the app layer's behavior as real Kameo actors (one Kameo actor per domain actor), each fed by a `tokio::spawn`ed "pump" task draining a transport inbox. This is the "engine-for-engine" replacement for the toy `LocalRuntime`, proving actors run autonomously with no manual `step()`.

The two **driven-port traits at the heart of the blocker** (`crates/kamiroh-ports/src/lib.rs`):

```rust
#![allow(async_fn_in_trait)] // spike scope: single-crate consumers, no dyn use yet

pub trait Transport {
    type Error: std::error::Error + Send + Sync + 'static;
    async fn send(&mut self, from: &Address, to: &Address, message: Message)
        -> Result<(), Self::Error>;
}

pub trait Inbox {
    async fn next(&mut self) -> Option<Delivery>;
}
```

Note the crate-level `#![allow(async_fn_in_trait)]` with the comment *"spike scope: single-crate consumers, no dyn use yet."* **The ports were explicitly designed for single-threaded consumption.** The kameo adapter is the first consumer that needs multi-threaded (`Send`) futures — and that assumption is exactly what breaks.

---

## What the kameo adapter does with those ports (the two `Send`-requiring sites)

1. **The pump** (`install()`): a `tokio::spawn`ed task that loops `while let Some(delivery) = inbox.next().await { pump_ref.tell(Deliver(delivery)).await … }`. `tokio::spawn` requires the future be `Send`, so `inbox.next()`'s future must be `Send`.

2. **The message handler** (`impl Message<Deliver> for Host`): inside `async fn handle(...)` it calls `self.transport.send(&self_address, &reply_to, ack).await`. kameo's `Message::handle` is declared `-> impl Future<Output = Self::Reply> + Send`, so `transport.send()`'s future must be `Send`.

Both `Inbox::next` and `Transport::send` return non-`Send`-bounded futures → both sites fail to compile.

---

## The 5 flagged API points — findings

The author left a doc-comment listing the 5 points "most likely to be wrong." Verified against the real kameo 0.22.2 source (read from `~/.cargo/registry/src/…/kameo-0.22.2/`):

| # | Flagged point | Verdict | Detail |
|---|---|---|---|
| 1 | `Actor` trait shape (`Args`/`Error`/`on_start`) | **No change** | `type Args = Self; type Error = Infallible; async fn on_start(args, ref) -> Result<Self, _>` matches kameo 0.22 exactly. The `type Error` bound is now `ReplyError`, but `std::convert::Infallible` satisfies it via the blanket `impl<T: Debug + Send + 'static> ReplyError for T`. |
| 2 | `kameo::spawn(actor)` vs `Actor::spawn(args)` | **CHANGED** | The free function `kameo::spawn` no longer exists. Spawning is now the `Spawn` trait method: `Host::spawn(host)` (needs `use kameo::actor::Spawn;`), returning `ActorRef<Host>` directly (not a `Result`/future). |
| 3 | `tell(msg).await` vs `.tell(msg).send().await` | **No change** | `.tell(msg).await` still works — `TellRequest` implements `IntoFuture` yielding `Result<(), SendError<M>>`, so the existing `.is_err()` check is fine. |
| 4 | `Context` path / lifetimes in `Message::handle` | **No change** | `kameo::message::Context<Self, Self::Reply>` and the `handle(&mut self, msg, ctx)` signature match. |
| 5 | `ActorRef::kill` / `stop_gracefully` naming | **No change** | `ActorRef::kill()` exists exactly as used in `stop()`. |

**So the author's blind guess was remarkably accurate** — 4 of 5 flagged points were already correct; only the `spawn` free-function → trait-method migration was real.

---

## The other in-adapter fix (mechanical, allowed)

Added `Sync` to the transport type parameter's bound across all where-clauses:

```
- T: Transport + Clone + Send + 'static,
+ T: Transport + Clone + Send + Sync + 'static,
```

Rationale: kameo requires `Actor: Send` and `Actor::Args: Send`. `Host` holds a `KameoRuntime` → `Arc<Inner<T, R>>`. `Arc<X>: Send` requires `X: Sync`. `Inner` has a bare `transport: T` field, so `Inner: Sync` requires `T: Sync`. This is a pure bound addition — no logic touched, no structure changed. The memory adapter's `MemoryTransport` (an `Arc<Mutex<…>>`) is `Sync`, so the test still satisfies it.

That fix cleared the `T cannot be shared between threads safely` errors, leaving only the two `future cannot be sent between threads safely` errors described above.

---

## The blocker, proven

kameo 0.22's trait definitions hardcode `Send`:

- `pub trait Actor: Sized + Send + 'static { type Args: Send; … }`
- `Message::handle(...) -> impl Future<Output = Self::Reply> + Send;`

The port methods are `async fn` in traits (return-position `impl Trait` in trait, "RPITIT"), which desugar **without** a `Send` bound and provide **no** way for a generic caller to require one. I confirmed with a minimal standalone repro on the pinned toolchain (`rustc 1.97.0`):

```rust
#![allow(async_fn_in_trait)]
trait Transport { type Error: Send + std::fmt::Debug; async fn send(&mut self) -> Result<(), Self::Error>; }
fn spawn_like<F: std::future::Future + Send + 'static>(_f: F) {}
fn use_it<T: Transport + Send + 'static>(mut t: T) where T::Error: Send {
    spawn_like(async move { let _ = t.send().await; }); // ERROR: future is not Send
}
```

The compiler's own suggestion is to **change the trait definition**:

```
help: `Send` can be made part of the associated future's guarantees for all implementations of `Transport::send`
3 - async fn send(&mut self) -> Result<(), Self::Error>;
3 + fn send(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
```

The one stable-Rust alternative that lives in the *caller* crate — **Return Type Notation** (`T: Transport<send(..): Send>`) — is still nightly-only (`error[E0658]: return type notation is experimental`, issue #109417). So there is genuinely no in-adapter, stable fix.

---

## The fix that works, and the evidence

Two lines in `crates/kamiroh-ports/src/lib.rs`:

```rust
// Transport
fn send(&mut self, from: &Address, to: &Address, message: Message)
    -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

// Inbox
fn next(&mut self) -> impl std::future::Future<Output = Option<Delivery>> + Send;
```

Properties:
- **Backward-compatible for implementors.** An `async fn` impl body still satisfies `fn -> impl Future + Send` as long as the body is actually `Send`, which the memory adapter's concrete `Arc<Mutex>`-based impls are. No adapter impls needed edits.
- **No impact on the app layer.** `LocalRuntime` *consumes* these ports single-threaded; tightening the trait only constrains *implementors*.
- **Not a redesign.** No types, fields, or control flow change — only the two method signatures.

**Verified result** (`cargo test --workspace` with this change applied):

```
harness_conversation ....... 2 passed
request_ack ................ 2 passed
kameo_conversation ......... 2 passed   (kameo_actors_run_the_harness_conversation, unadmitted_commands_are_dropped_by_kameo_hosts)
adapter-memory (unit) ...... 5 passed
app (unit) ................. 10 passed
domain (unit) .............. 9 passed
------------------------------------------------
TOTAL ...................... 30 passed, 0 failed
```

Both `kameo_conversation` tests pass **as written** — no assertions weakened. After verifying, I **reverted** the ports change to respect the "don't touch other crates" constraint, so the working tree currently holds only the adapter changes (and therefore does **not** compile).

---

## The tension for the advisor to weigh

The fix is objectively minimal, safe, and verified. But it collides with two explicit instructions:

1. **"do NOT touch other crates' code."** The fix is in `kamiroh-ports`, not the adapter.
2. **"if the real API forces a structural change, STOP and report."** Is a driven-port trait-signature change "structural"? It's 2 lines and backward-compatible — but it *does* alter the hexagon's port contract, which is arguably the most architecturally significant surface in the codebase.

Reasonable arguments each way:

- **For applying it:** It's the compiler-endorsed, idiomatic fix; it's backward-compatible; it reflects a real latent design gap (`#![allow(async_fn_in_trait)]` was a "spike scope" shortcut that a multi-threaded adapter was always going to force). The whole point of adding the kameo adapter is to run actors on real threads; requiring `Send` on driven-port futures is a legitimate, permanent consequence of that goal, not a kameo-version quirk. `Send` bounds on a transport/inbox port are conventional in async Rust.
- **Against applying it unilaterally:** The task author drew a hard boundary and pre-authorized a STOP for exactly this shape of situation. Changing a shared port contract touches every current and future adapter and the app layer's expectations; it deserves an explicit human decision and possibly an ADR, not a silent `fix:` commit. There may be a deliberate reason the ports are `Send`-agnostic (e.g., a planned single-threaded `!Send` transport, or `dyn`-compatibility concerns).

---

## Options on the table

| Option | What happens | Trade-off |
|---|---|---|
| **A. Apply the ports `+ Send` change** | Add the 2-line bound to `kamiroh-ports`, then continue: `fix:` commit (noting the ports touch), `cargo vendor`, offline verify, push branch. | Green build, minimal diff, verified. Crosses the "no other crates" line — needs sign-off + arguably an ADR / commit-message note. |
| **B. Adapter-only, stop** | Keep only adapter changes (spawn, `Sync` bound, dep bump). It won't compile. Don't commit/vendor/push. | Honors the boundary literally. Leaves the branch red; the task's green-build goal is unreachable under the stated constraints. |
| **C. Push the port design back** | Reconsider whether `kamiroh-ports` should expose a `Send` (or `?Send`) variant deliberately — e.g., split into `Send` and local variants, or use `trait_variant`. | Cleanest long-term if the hexagon will host both threaded and single-threaded adapters. Larger scope than "minimal & mechanical"; a design task, not a fix. |

---

## The specific question

**Given that the only path to a green `cargo test --workspace` is a 2-line, backward-compatible `+ Send` bound on `Transport::send` and `Inbox::next` in `kamiroh-ports` (a crate the task said not to touch), should the session (A) apply it and continue, (B) stop at the boundary with a red build, or (C) treat the port `Send` contract as a design decision to be made deliberately?**

Supporting facts for the decision:
- 4 of 5 flagged kameo API points were already correct; the adapter logic is sound.
- The change is verified to produce 30/30 passing tests with no weakened assertions.
- No existing implementor (memory adapter) or consumer (app `LocalRuntime`) requires edits.
- The ports crate already carries a `// spike scope` allow-attribute acknowledging this shortcut.
- Everything downstream (commit, vendor, offline verify, push) is mechanical once the build is green.

---

## Appendix: exact current working-tree state

- `crates/kamiroh-adapter-kameo/Cargo.toml`: `kameo = "0.17"` → `kameo = "0.22"`.
- `crates/kamiroh-adapter-kameo/src/lib.rs`:
  - `use kameo::actor::ActorRef;` → `use kameo::actor::{ActorRef, Spawn};`
  - `let actor_ref = kameo::spawn(host);` → `let actor_ref = Host::spawn(host);`
  - `T: Transport + Clone + Send + 'static` → `… + Send + Sync + 'static` (all where-clauses).
- `Cargo.lock`: updated by the dependency bump.
- `crates/kamiroh-ports/src/lib.rs`: **unchanged** (the `+ Send` fix was applied for verification, then reverted).
- Nothing committed. Branch still at `78a989d`.
