# Completion brief: iroh 1.0 transport adapter — done, pushed, awaiting review

**For:** Mez (Claude Cowork / Fable), design session for spike-1.
**From:** the Claude Code session on `cowork/iroh-transport`.
**Re:** the blind-written Iroh transport adapter's first local build.
**Date:** 2026-08-12
**Status:** ✅ Complete. Branch pushed. `master` untouched (no merge). No PR opened.

---

## TL;DR

The Iroh transport adapter (`crates/kamiroh-adapter-iroh`) was bumped from the
assumed `iroh 0.35` to the real latest **`iroh 1.0.3`** — a major-version jump.
Of the **7 assumption points**, **3 drifted and 4 held**. All fixes were
mechanical API adaptation; **no STOP was needed** — nothing forced a structural
change, so unlike the kameo round there was no advisory to escalate. The
integration test `tests/iroh_conversation.rs` needed **zero edits** and passes
**as written**, over real loopback QUIC. Full workspace: **38 tests, 0 failures**,
verified online and offline. Vendor grew **28 MB → 559 MB**.

---

## The 7 assumption points (iroh 0.35 → real 1.0.3)

| # | Point | Changed? | Finding |
|---|---|---|---|
| 1 | Endpoint builder | **Yes** | `Endpoint::builder()` now takes a `Preset` → `builder(presets::Minimal)` (crypto provider only; no relay/discovery, matching decision 19). `endpoint.node_id()` → `endpoint.id()`. |
| 2 | `node_addr()` watcher | **Yes** | `node_addr().initialized()` gone; `addr()` is a getter over `watch_addr()`. Adapter now waits for the first non-empty direct-address set so the peer book never caches an undialable address. |
| 3 | `connect` args | No | `connect(impl Into<EndpointAddr>, &[u8])` — existing `connect(addr, ALPN)` works. |
| 4 | `remote_node_id` | **Yes** | Renamed `remote_id()`, and on an established `Connection` now **infallible** (`EndpointId`, not `Result`). |
| 5 | Stream API | No | `open_uni`/`write_all`/`finish()`[sync]/`read_to_end`/`stopped`, `accept_uni` all match. |
| 6 | Accept loop | No | `accept().await` → `Option<Incoming>` → `.await` → `Connection` — existing shape. |
| 7 | Key types | No | `SecretKey::from_bytes(&[u8;32])` infallible; `EndpointId` (= `PublicKey`) keeps hex `Display`/`FromStr`. |

**Cross-cutting rename (spans 2/3/7):** iroh renamed `NodeId`→`EndpointId` and
`NodeAddr`→`EndpointAddr` — the former **colliding with the domain's own
`EndpointId`**. Aliased back on import (`EndpointAddr as NodeAddr`,
`EndpointId as NodeId`) so the routing/framing/peer-book body stayed textually
unchanged. One field rename: `addr.node_id` → `addr.id`.

Calibration note: the blind-written guesses again held up well (4/7 exact; the
3 that moved were the entry-point builder, the address getter, and one
fallibility change — all the "surface API churns most" category, consistent with
the kameo round's 4/5). This continues to be a good predictor for how the next
adapter should be written.

---

## Trust rule — preserved exactly

`Delivery::from.endpoint` is still constructed from `connection.remote_id()` —
the connection's authenticated remote key — **never** from frame content. Only
the name halves ride in the postcard frame. The `remote_id()` rename did not
weaken this: it's the same authenticated-key source, now infallible. No change to
routing, framing, the peer book, or any test assertion.

---

## What shipped

**Adapter (`fix(adapter-iroh)`):**
- `iroh = "0.35"` → `"1"`.
- Import aliases + `use iroh::endpoint::presets` + `use iroh::Watcher`.
- `Endpoint::builder(presets::Minimal)`; `endpoint.id()`.
- `addr()` waits on `watch_addr()` for a dialable direct address.
- `add_peer`: `addr.id`.
- accept loop: `connection.remote_id()` (infallible).
- `tests/iroh_conversation.rs`: **no edits** — the adapter's public surface stayed
  textually stable through the aliases, so the test compiled and passed unchanged.

**Vendor (`chore(vendor)`):**
- `cargo vendor vendor` after the bump. `.cargo/config.toml` unchanged.

---

## Verification

- `cargo test --workspace` (online): **38 passed, 0 failed.**
- `cargo test --workspace --offline` from a wiped `target/`: **38 passed, 0 failed** —
  hermetic build from vendored sources, including both real-QUIC iroh tests.
- Headline test (`actors_converse_across_real_iroh_endpoints`): ping→pong, remote
  spawn, a turn exchange with **ack-before-answer**, and a 2-round countdown — all
  over loopback QUIC between two endpoints.
- Denial test (`unadmitted_endpoints_are_denied_across_the_wire`): stranger C's
  spawn reaches B's socket, is dropped silently (C hears only silence), and
  admitted A gets `Failed("no such actor")` proving the spawn never happened.

**Observed wall-time (real QUIC handshakes now):**
- iroh_conversation suite: **~2.1 s** (2 tests; QUIC handshake/connection-setup
  bound — the in-memory suites run in ~0.00–0.2 s).
- Full workspace: ~5.9 s online; ~25.5 s offline-from-clean (the delta is
  compiling the large vendored iroh tree, not test execution).

---

## Vendor growth

**28 MB → 559 MB** (31 → 386 crates). iroh pulls `iroh-base`, `iroh-relay`, the
`noq` QUIC stack, rustls/ring, quinn-udp, and a long transitive tail. This is the
vendored-source cost decision 14 accepts for hermetic offline builds. Worth a
heads-up: the fork's history now carries a half-gigabyte of vendored source, and
each future heavy-dep bump re-vendors on top. If that history weight becomes a
concern, options for a later decision: a shallow/vendor-on-a-side-branch scheme,
`cargo vendor --no-delete` hygiene, or gitignoring vendor/ and reconstructing it
in CI. Out of scope here — flagging, not acting.

---

## Commits (on `cowork/iroh-transport`), and state

```
3041102 chore(vendor): re-vendor for iroh 1.0 offline builds
aae8fa6 fix(adapter-iroh): adapt to iroh 1.0
b4fc081 feat: Iroh transport adapter (blind-written; first build happens locally)  <- prior HEAD
```

- Branch **pushed**: `origin/cowork/iroh-transport`. **Not merged** into master.
- `master` is in sync on the remote at `6fda45f` ("Merge upstream …"), which
  already contains the earlier `fe9f4b9` party-phone-turn merge as an ancestor —
  so pushing master this round was a no-op. Nothing rewound.
- **No PR opened.** GitHub printed its usual create-a-PR suggestion on push;
  left for you/the humans to open when review's ready.
- Housekeeping: the stray untracked `.DS_Store` in the repo root persists
  (pre-existing, not from this work); kept out of every commit.
- Method note for the mechanically-minded: to fetch and re-vendor the new iroh
  tree I temporarily set `.cargo/config.toml` aside (the vendored-sources
  replacement blocks crates.io), did the online work, then restored it and
  re-verified fully offline. The config file is committed unchanged.
