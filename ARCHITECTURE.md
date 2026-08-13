# kamiroh — Architecture (Spike 1)

**Status:** Accepted (spike scope)
**Date:** August 12, 2026
**Deciders:** Casey Bowman

This document records the architecture for the second architectural spike of kamiroh
(the `kamiroh-workshop-1` fork). It is designed from scratch, independent of spike 0.

## Intent

kamiroh combines actors at each end of an internet conversation. Actors are implemented
with [Kameo]; conversations travel over [Iroh]. Where an AI agent participates, one actor
is dedicated to that agent as its communications proxy — everything the agent says or
hears in this system flows through its actor. A conversation may run agent↔agent,
agent↔app, or app↔app; it may be a single request or a long-lived exchange; and either
end may be an application embedding part of kamiroh as a library.

[Kameo]: https://crates.io/crates/kameo
[Iroh]: https://crates.io/crates/iroh

## Shape

A **modular monolith**: one deployable unit, organized as a Cargo workspace whose crate
boundaries enforce the **ports-and-adapters** (hexagonal) structure. Dependencies point
inward only; the domain compiles with no knowledge of Kameo, Iroh, or serialization
formats.

## Glossary — the layering of terms

From the wire up. Fixing these words early is deliberate: in a
ports-and-adapters design the ubiquitous language is the architecture.

- **Connection** — infrastructure, endpoint↔endpoint. The Iroh QUIC pipe (or
  nothing at all, in the memory adapter). Owned entirely by the transport
  adapter: reconnects, multiplexing, lifetimes. The domain never says this
  word.
- **Conversation** — domain, actor↔actor. The ongoing relationship between two
  Addresses, long- or short-lived. It *spans* connections: if the wire drops
  and returns, it is the same conversation. Admission guards it delivery by
  delivery. A conversation begins implicitly with its first admitted delivery —
  there is no opening handshake in v0.
- **Protocol** — the rulebook, not an instance: a named legal sequence of
  vocabulary messages (request-ack, harness), including what opens an exchange
  and what completes one. Reusable across any conversation.
- **Exchange** — one complete run of a protocol within a conversation, from its
  opening message to the protocol's terminal state — however many round trips
  the protocol defines. Request-ack is the degenerate two-message case. A long
  conversation is a series of exchanges, one protocol after another. In v0 a
  conversation runs **one exchange at a time**.
- **Turn** — one unit of party-level messaging in the `turns` protocol: "here
  is my answer to what you asked; here is what I now ask." An exchange of
  turns alternates strictly: opened by a request-only turn, continued by
  answer+request turns, concluded by an answer-only turn. One incoming turn =
  one atomic party state change = at most one outgoing turn, emitted only
  after the state settles.
- **Party** — the app-implemented brain behind an actor: the trait an
  embedding application implements to receive turns (pushed by kamiroh).
- **Phone** — the live handle an app holds on one conversation: opening it is
  purely local; it sends turns and enforces alternation on both directions.
- **Vocabulary** — the words themselves: the closed set of message kinds from
  which protocols are built.

## Domain model

The domain crate holds:

- **Endpoint** — an Iroh endpoint identity (a public key). The unit of transport-proven
  identity.
- **Hex** — hex-string value objects for keys and identifiers.
- **Secret** — secret-key material backing an endpoint, handled as a domain value with
  care taken not to leak it through `Debug`/logs.
- **ActorName** — a name unique *within* an endpoint.
- **Address** — the pair (Endpoint, ActorName). How one actor designates another.
- **Actor** — the domain concept of a named communicating party at an endpoint
  (distinct from Kameo's actor type, which implements it in the runtime adapter).
- **Allowlist** — per-actor inbound policy; see Trust model.
- **Conversation** — the ongoing actor↔actor relationship (see Glossary), with
  app-layer state tracking the current exchange.
- **Exchange** — one run of a protocol within a conversation (see Glossary).
- **Vocabulary** (module) — the constrained set of message kinds actors may exchange.
  Agnostic to the kind of agent (or non-agent) behind either end.
- **Protocol** — a named, legal sequence of vocabulary messages between two parties,
  each party opaque (agent or embedding app, one side or both).

## Trust model

The two halves of an Address carry different kinds of trust:

- An **Endpoint** is a public key. Iroh proves, cryptographically, which endpoint a
  connection comes from.
- An **ActorName** is *claimed* by the remote runtime, not proven. Names are addressing,
  not authentication.

Consequently the **allowlist holds endpoints only**: admitting an endpoint means
trusting that endpoint's runtime, including its honesty about which of its actors is
speaking. Allowlist semantics:

- **Deny by default** — an actor with an empty allowlist receives nothing.
- **Checked per delivery**, not only at conversation-open, so a long-lived connection
  cannot outlive a revocation.

## Vocabulary v0

A closed, compile-time set (Rust enums) shared by both ends. Wire encoding is an
adapter concern, not a domain one.

- **Request** — payload addressed to the party behind an actor.
- **Ack** — delivery acknowledgment from the remote *actor*: "the request reached the
  agent's dedicated actor and was handed over." Deliberately distinct from any future
  `Response` (the party's actual answer), so response semantics can arrive later
  without remodeling.

- **Response** — the party's actual answer to a Request, distinct from Ack.
- **Turn** — `Open { request }` / `Continue { response, request }` /
  `Close { response }`: the enum encodes that a turn is never empty.

Protocols in v0:

- **request-ack** — the first and simplest protocol: one Request, one Ack.
- **turns** — party-level conversation in strictly alternating Turns, tracked
  by the `TurnState` machine on both sides (decision 17). Runtimes ack a
  turn's request half on handover to the party — the fast receipt while the
  party thinks; a `Close` gets no ack in v0 (deferred reliability work).
- **harness** — a minimal lifecycle/test protocol: spawn a named actor, stop it,
  ping it. Its exchanges are command/reply pairs: `Spawn → Spawned`,
  `Stop → Stopped`, `Ping → Pong`, with `Failed` as the error reply to any
  command. It exists so integration tests can orchestrate both ends of a real
  Iroh conversation using the system's own machinery — and it doubles as proof
  that the protocol abstraction generalizes beyond request-ack. Admitting an
  endpoint to `harness` is a privileged grant; the full agent-control
  vocabulary is deliberately deferred.

## Hexagon

**Core (inside):**

- `kamiroh-domain` — the model above; pure, sync, dependency-light.
- `kamiroh-app` — application services: conversation lifecycle, routing inbound
  deliveries to the right actor, allowlist enforcement, protocol state.

**Ports (`kamiroh-ports`, their own crate):**

The app-facing boundary (the "1A boundary") is exactly two surfaces
(decision 16):

- **`Party`** (driven, push) — the trait the embedding app implements per
  actor; kamiroh drives it with incoming turns. Its signature is the
  atomicity contract (decision 17).
- **`Phone`** (driving handle, in `kamiroh-app`) — how an app opens
  conversations and sends turns; alternation-enforcing.

The kamiroh↔engine boundary (the "1B boundary") stays internal plumbing —
`Transport`, `Registry`, `Inbox`, and the runtimes' hosting contract — and
apps never see or name it:

- *Driven* — `Transport`: open/accept conversations to an Address, send/receive
  vocabulary messages. Defined by the core, implemented by adapters.
  `Registry`/`Inbox`: local actor binding and the pull surface the runtimes'
  pumps drain.

Putting the port traits in a dedicated crate means *driven* adapters depend on
`kamiroh-domain` + `kamiroh-ports` only — never on the application layer — so the
hexagon's dependency arrows are enforced by the compiler, not convention.

**Adapters (outside, named `kamiroh-adapter-*`):**

Adapters come in two kinds, and the dependency rule differs:

- *Driven* adapters are called **by** the core through ports and stay app-blind:
  - `kamiroh-adapter-iroh` — implements `Transport`/`Registry` on Iroh
    connections; owns endpoint setup, connection lifetimes (short- or
    long-lived), and the wire codec.
  - `kamiroh-adapter-memory` — an in-process `Transport`/`Registry` for tests:
    exercises the core with no network involved.
- *Driving* adapters call **into** the application — like a web framework
  hosting handlers — and so legitimately depend on `kamiroh-app`:
  - `kamiroh-adapter-kameo` — animates domain Actors as Kameo actors:
    mailboxes, supervision, the dedicated-actor-per-agent pattern, hosting the
    app layer's inbound processing and harness execution.
- Agents themselves live **outside** the hexagon, on the driving side, behind their
  dedicated actors.

## Workspace layout

```
kamiroh/                      # workspace root; root crate `kamiroh` is the facade
├── Cargo.toml                # [workspace] + the facade package
├── src/                      # facade: re-exports, wiring, prelude for embedders
└── crates/
    ├── kamiroh-domain/
    ├── kamiroh-ports/
    ├── kamiroh-app/
    ├── kamiroh-adapter-iroh/
    ├── kamiroh-adapter-kameo/
    └── kamiroh-adapter-memory/
```

The root `kamiroh` crate keeps the published name and crates.io metadata, and is what
embedding applications depend on.

## Testing strategy

- Domain and application logic: unit tests, no I/O.
- Integration: two real Iroh endpoints in one test process, orchestrated over the
  `harness` protocol — spawn an echo actor on the far side, run request-ack through
  it, stop it, assert allowlist denials for unadmitted endpoints.

## Decision log

1. **Modular monolith, Cargo workspace, ports-and-adapters.** One unit to build and
   reason about at spike scale; crate boundaries make the hexagon compiler-enforced
   rather than conventional.
2. **Allowlist checks endpoints only.** Names are unauthenticated claims; a policy
   keyed on (endpoint, name) would imply a guarantee the transport cannot provide.
3. **Deny by default; enforce per delivery.** Empty list means silence; revocation
   takes effect on live connections.
4. **Ack ≠ Response.** v0 confirms delivery to the dedicated actor only. Response
   semantics (correlation, timeouts, partials, reconnect-after-answer) are expected
   to be subtly complicated and are deferred as their own design step — last.
5. **Vocabulary v0 is closed.** Both ends compile against the same enums; versioning
   and cross-build evolution deferred until a second consumer exists.
6. **Minimal harness protocol now; agent control later.** Just enough lifecycle
   (spawn/stop/ping) to let tests drive both ends; the general "control a remote
   agent" vocabulary waits for a real security design.
7. **Kameo and Iroh are adapters.** The domain speaks of Actors and Conversations;
   the crates that realize them are replaceable at the edges.
8. **Ports get their own crate; adapters are named `kamiroh-adapter-*`.** A layout
   convention carried over from spike 0 at Casey's direction (structure only — no
   other spike-0 design is imported). It lets adapters compile against domain +
   ports without seeing the application layer, and makes the adapter roster legible
   at a glance.
9. **"Exchange" is a first-class term.** One complete run of a protocol within a
   conversation, spanning as many round trips as the protocol defines. It names
   where protocol state lives: the app layer tracks, per conversation, the
   current exchange and its progress.
10. **One exchange at a time per conversation (v0).** Strictly sequential;
    interleaved concurrent exchanges drag in correlation machinery that belongs
    with the deferred response-semantics work.
11. **No opening handshake (v0).** Admission is checked per delivery, so a
    handshake adds no security; a conversation begins implicitly with its first
    admitted delivery. A hello/capability protocol can slot in later as just
    another protocol if wanted.
12. **Local actor binding is a port.** The runtime asks the transport to bind an
    Address and receives that actor's Inbox; dropping the Inbox unbinds. The
    memory net implements it as registration; the Iroh adapter will implement it
    as routing inside the endpoint.
13. **Driving adapters may depend on the app layer; driven adapters may not.**
    Refines decision 8, which was written with driven adapters in mind. The
    Kameo runtime is a driving adapter — its whole job is hosting application
    behavior (inbound processing, harness execution) inside real actors — so it
    depends on `kamiroh-app`, exactly as a web framework depends on the handlers
    it hosts. Transport adapters remain app-blind.
14. **Dependencies are vendored.** The cloud workspace cannot reach crates.io,
    so `cargo vendor` output and `.cargo/config.toml` are committed once heavy
    deps (kameo, tokio, iroh) land. Cost: vendored source in the fork's history.
    Benefit: hermetic offline builds everywhere, cloud included.
15. **Driven-port futures are `Send`.** `Transport::send` and `Inbox::next`
    return `impl Future + Send`, stated explicitly in the trait (RPITIT with
    a bound) rather than via `async fn`. Surfaced by the first multi-threaded
    consumer (the Kameo runtime, whose engine requires `Send` handler
    futures) — but adopted as a fact about the system, not a kameo
    accommodation: these ports exist to be crossed by threads. The former
    `#![allow(async_fn_in_trait)]` "spike scope" shortcut is retired. A
    `?Send`/single-threaded variant is deliberately not provided until a
    single-threaded embedder exists to justify it. (Full deliberation:
    `docs/advisories/2026-08-12-kameo-ports-send-*.md`.)
16. **The app boundary is two surfaces: Party and Phone.** An embedding app
    implements `Party` (one per actor — the brain behind it, driven by
    kamiroh, push not pull) and holds `Phone`s (the driving handle: open a
    conversation locally, send turns). The kamiroh↔engine boundary stays
    internal plumbing apps never see. Opening a conversation remains
    handshake-free (decision 11): constructing a Phone is a local act.
17. **Turns are the unit of party-level messaging; exchanges alternate
    strictly.** A turn couples "answer to your outstanding request" with
    "optionally, my next request"; the `Turn` enum (Open/Continue/Close)
    makes an empty turn unrepresentable. One incoming turn = one atomic
    party state change (enforced by `Party::on_turn(&mut self, …)` and
    per-actor mailbox serialization) = at most one outgoing turn, sent only
    after the handler returns. Strict alternation (the `TurnState` machine,
    held by both sides and enforced by runtimes and Phones) collapses
    response correlation: exactly one request is outstanding per exchange, so
    `RequestId` is audit/timeout material, not disambiguation. The delivery
    `Ack` stays distinct (decision 4): runtimes ack a turn's request half on
    handover, before the party thinks. Deferred: acks for `Close` turns,
    timeouts, disconnect mid-exchange, streaming/partial responses.
18. **Wire encoding is postcard over a feature-gated serde.** The domain
    stays dependency-free by default; the `serde` feature adds derives to the
    vocabulary (with `ActorName` deserializing through its validating
    constructor). Wire adapters enable the feature and choose the format —
    the Iroh adapter uses postcard (compact, serde-native). Format choice
    stays adapter-local; nothing outside an adapter may depend on it.
19. **Iroh adapter v0: static peer book, one frame per uni-stream, origin
    from the connection.** Endpoint-id→address resolution is an explicit
    peer book (`add_peer`), per the deferred-discovery decision. Each message
    travels as one length-delimited postcard frame `{from_name, to_name,
    message}` on a fresh uni-stream over a cached per-peer connection (one
    retry on stale connections); the ALPN is `kamiroh/0`. The receiving
    adapter constructs `Delivery::from` with the endpoint taken from the
    connection's authenticated remote key — never from frame content — and
    only the name halves ride in the frame. Relays and discovery are
    disabled in tests (loopback direct addresses); production relay policy
    is deferred.
20. **Vendored sources live on an artifact branch; publication is a history
    boundary.** Refines decision 14 after the iroh tree took `vendor/` to
    ~559 MB: committed blobs ride ancestry-preserving merges forever, so
    `master` now gitignores `vendor/` and `.cargo/`; the orphan
    `vendor-snapshot` branch (force-pushed, merged into nothing) carries
    them for the cloud session's hermetic builds. The workshop's existing
    vendor history stays its private cost: graduation to staging publishes
    a fresh vendor-free snapshot branch — a deliberate content-not-ancestry
    boundary, carved out from the cross-tier merge-commit rule, justified
    because a workshop fork is archival once its spike graduates. Within
    staging and staging→main, ancestry-preserving merges remain mandatory.
    (Full guide: `docs/VENDORING.md`.)

21. **Internet-facing operation uses n0's public infrastructure; hermetic
    stays the default.** `NetProfile::Hermetic` (relay-less, lookup-less,
    static peer book) remains what tests and closed deployments get, and is
    the default. `NetProfile::N0` (`presets::N0`) turns on n0's relay fleet
    and address publishing/lookup: peers dial by endpoint id alone, and NAT
    traversal — hole-punching with relay fallback — is Iroh's job, exactly
    as designed. Consequences owned: an N0 endpoint publishes a signed
    address record to n0's public lookup service, and internet operation
    depends on n0's infrastructure (self-hosted relays deferred until
    wanted). The relay-less apparatus (fixed-port binding, leased
    port-forwards via scripts/internet-check-serve.sh) remains in-tree as
    the documented fallback for single-NAT setups an operator controls —
    and as the boundary marker of what relay-less operation cannot reach:
    multi-NAT and CGNAT'd hosts, which need N0.

## Deferred

- Response semantics (the subtle part, saved for last — see decision 4).
- Agent-control vocabulary beyond the test harness.
- Discovery: how initiators learn Addresses (static configuration for the spike).
- Name authentication within an endpoint, if it is ever wanted.
- Vocabulary versioning across differing builds.
- Wire format selection (serde-compatible; chosen inside the transport adapter).
