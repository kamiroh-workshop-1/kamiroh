# kamiroh

Kameo actors for agents over Iroh

Peer actors, addressable by name and endpoint, that message each other—locally or across the network—to drive agents.

*kamiroh — "Let's be awesome!"*

> **Status: architectural spike, working end to end** — API and behavior may
> change, but the full stack runs today: real QUIC between real machines
> across the real internet, no server in between and no path configuration.

## Aims

kamiroh puts two parties in conversation across the internet without a server
in between. Each side of a conversation is an **actor** — a named party living at an
[Iroh](https://www.iroh.computer/) endpoint. One endpoint can host many actors, each
with its own unique name, and any actor can open a conversation with another by naming
its endpoint and actor name.

Where an AI agent takes part, one actor is dedicated to that agent as its
communications proxy: everything the agent sends or receives flows through its actor.
But agents are optional — either end of a conversation can just as well be an
application embedding kamiroh as a library. Conversations may be one quick
request-and-acknowledgment or a long-lived back-and-forth, following small, defined
**protocols** built from a constrained, agent-agnostic vocabulary.

Security is allowlist-based and deny-by-default: an actor receives messages only from
endpoints it has explicitly admitted, and an empty allowlist means silence. What an
endpoint *is* is proven cryptographically by the connection itself — names are claimed,
endpoints are proven, and admission decisions rest only on the proven part.

Under the hood, kamiroh is a modular monolith in the ports-and-adapters style — a Rust
workspace whose core knows nothing about the network or the actor runtime, with
[Kameo](https://crates.io/crates/kameo) animating the actors and Iroh carrying the
conversations. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full picture.

## What works today

The whole path, thin but real: turn-taking conversations with delivery
acknowledgments distinct from answers, a strict alternation state machine that
makes illegal turn sequences unrepresentable, per-actor allowlists checked on
every delivery, and a small harness protocol (ping, remote spawn) for driving it
all from tests and demos.

Three interchangeable transports carry the same application core, which is the
ports-and-adapters claim made checkable: an in-memory transport with zero
dependencies, a [Kameo](https://crates.io/crates/kameo) runtime hosting actors,
and an [Iroh](https://www.iroh.computer/) adapter speaking QUIC. The Iroh
adapter offers two network profiles — `Hermetic` (the default: no external
infrastructure, explicit peer addresses) and `N0` (dial by endpoint id alone;
n0's relays and address lookup handle NAT traversal, hole-punching a direct
path where possible with relay fallback).

The `N0` profile is field-validated: a laptop on a phone hotspot and again on
café wifi conversed with a machine behind multiple NAT layers, dialing by id
only, and got a hole-punched **direct path** both times
([the brief](docs/briefs/2026-08-13-internet-check-brief.md)). The test suite —
39 tests, from domain invariants through real-QUIC loopback conversations —
runs hermetically, with no network at all.

## Try it in five minutes, one machine

Two terminals, the demo identities from the repo (world-readable on purpose —
never reuse them for anything real):

```sh
cargo build --release --example harness_ping -p kamiroh-adapter-iroh

# Terminal A — serve, admitting the demo checker:
./target/release/examples/harness_ping serve \
  --secret 0202020202020202020202020202020202020202020202020202020202020202 \
  --allow 8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c
# note the port in the ADDR lines, wait for READY

# Terminal B — check: ping, remote-spawn an echo actor, hold a turn exchange:
./target/release/examples/harness_ping check \
  --secret 0101010101010101010101010101010101010101010101010101010101010101 \
  --peer-id 8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394 \
  --peer-ip 127.0.0.1:<port-from-ADDR>
```

`PONG`, `SPAWNED`, `TURN OK (ack seen: true)`, `CHECK PASSED` — that's the
whole protocol stack over real QUIC on your loopback. For the same thing
across the actual internet, see [docs/INTERNET-CHECK.md](docs/INTERNET-CHECK.md);
between containers, [docs/INCUS-CHECK.md](docs/INCUS-CHECK.md).

## The shape of the workspace

The dependency arrows all point inward: `kamiroh-domain` (endpoints, names,
allowlists, the message vocabulary, the turn-taking state machine — pure, no
dependencies) and `kamiroh-ports` (the traits the core speaks through:
`Transport`, `Inbox`, `Registry`, `Party`) know nothing of the outside.
`kamiroh-app` implements conversations, admission, and the harness against
those ports. The three `kamiroh-adapter-*` crates — `memory`, `kameo`,
`iroh` — each plug the same core into a different world. Cross-crate behavior
lives in the workspace-level `tests/`.

Embedding kamiroh in your own application — including wrapping a foreign
runtime's actors as parties — is covered in [docs/EMBEDDING.md](docs/EMBEDDING.md).

## Reading guide

[ARCHITECTURE.md](ARCHITECTURE.md) is the source of truth: a glossary of the
precise terms (conversation, exchange, turn, party, phone…) and a numbered log
of every design decision with its reasoning. [docs/WORKFLOW.md](docs/WORKFLOW.md)
describes how this repo is actually developed — a human and two AI agents in
named roles, with branch namespaces and verification habits.
[docs/VENDORING.md](docs/VENDORING.md) explains the `vendor-snapshot` branch
(never delete it). Runbooks live in `docs/`, results in `docs/briefs/`, and
contested design moments keep their full deliberation in `docs/advisories/`.

## Status

An architectural spike: the concepts above exist and are tested, but the API is
young and will change. Not yet here: timeouts, disconnect handling mid-exchange,
streaming, and the agent-control vocabulary — see the decision log for what's
deliberately deferred.

## License

MIT
