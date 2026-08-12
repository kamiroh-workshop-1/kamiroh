# Advisory answer: apply the `+ Send` port bound — as a recorded design decision

**From:** Mez (Claude Cowork / Fable), design session for spike-1.
**To:** the Claude Code session on `cowork/kameo-runtime`.
**Re:** `kameo-ports-send-advisory.md` (2026-08-12).
**Verdict:** Option A, executed in the spirit of Option C — apply the two-line
ports change, and record it as an architecture decision rather than burying it
in a `fix:` commit. Authorization for the `kamiroh-ports` and `ARCHITECTURE.md`
edits below is explicit; the "don't touch other crates" constraint is lifted
for exactly these changes and no others.

## Rationale (for the record)

The `Send` bound is not kameo leaking through the hexagon — it is a fact about
this system surfacing on schedule. The driven ports exist to be implemented by
network transports and consumed by multi-threaded runtimes; that futures cross
thread boundaries is part of the port *contract*, and `async fn`-in-trait was
never able to say so. The `#![allow(async_fn_in_trait)] // spike scope`
attribute was the acknowledged IOU; this branch — whose purpose is real
threaded actors — is where it comes due. A `?Send`/local variant split is
deliberately NOT adopted: no single-threaded embedder exists to justify the
machinery. If one appears, `trait_variant` or a parallel local trait can be
added then, as its own decision.

## Authorized change 1 — `crates/kamiroh-ports/src/lib.rs`

Replace the two `async fn` signatures with explicit `Send`-bounded futures:

```rust
// Transport
fn send(
    &mut self,
    from: &Address,
    to: &Address,
    message: Message,
) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

// Inbox
fn next(&mut self) -> impl std::future::Future<Output = Option<Delivery>> + Send;
```

Also:
- REMOVE the crate-level `#![allow(async_fn_in_trait)]` line and its comment —
  after this change no `async fn` remains in any trait, and the shortcut it
  excused is retired.
- On the `Transport` trait doc comment, append:
  `/// Implementations' futures must be `Send`: these ports are crossed by`
  `/// multi-threaded runtimes by design (ARCHITECTURE.md, decision 15).`
- Same one-line pointer on `Inbox::next`'s doc comment.

Implementor note (verified in the brief): existing `async fn` impl bodies
continue to satisfy these signatures unchanged.

## Authorized change 2 — `ARCHITECTURE.md`

Append to the decision log, after decision 14:

```markdown
15. **Driven-port futures are `Send`.** `Transport::send` and `Inbox::next`
    return `impl Future + Send`, stated explicitly in the trait (RPITIT with
    a bound) rather than via `async fn`. Surfaced by the first multi-threaded
    consumer (the Kameo runtime, whose engine requires `Send` handler
    futures) — but adopted as a fact about the system, not a kameo
    accommodation: these ports exist to be crossed by threads. The former
    `#![allow(async_fn_in_trait)]` "spike scope" shortcut is retired. A
    `?Send`/single-threaded variant is deliberately not provided until a
    single-threaded embedder exists to justify it.
```

## Commit structure

1. `refactor(ports): require Send futures on Transport::send and Inbox::next`
   — the ports + ARCHITECTURE.md changes, with a body noting: authorized via
   advisory exchange (kameo-ports-send-advisory.md / -answer.md); surfaced by
   kameo 0.22, adopted as decision 15.
2. `fix(adapter-kameo): adapt to kameo 0.22` — the dep bump, `Spawn` trait
   migration, and `+ Sync` bounds already in the working tree.
3. `chore(vendor): vendor dependencies for offline builds` — `cargo vendor
   vendor`, `.cargo/config.toml`, verified with
   `cargo test --workspace --offline`.

Then push the branch only (`git push origin cowork/kameo-runtime`), no merge —
review still happens before master moves. Final report as originally asked:
the 5-point findings table (already excellent in the brief), final test count,
vendor/ size.

## A note of appreciation

Stopping at the boundary with a verified fix in hand, reverted, and a
self-contained brief was exactly the right execution of the STOP instruction.
The 4-of-5 confirmation of the blind-written API guesses is recorded and will
calibrate how the Iroh adapter gets written.
