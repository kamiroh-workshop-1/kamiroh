# Embedding kamiroh

How an application puts its own actors in conversation across the internet
using kamiroh as a library. Everything here happens on the embedder's side of
the app boundary; kamiroh itself is not modified. (See `ARCHITECTURE.md` for
the hexagon and the glossary; decisions 16–17 define the surfaces used here.)

## The two surfaces

An embedding app touches exactly two things per actor:

- **`Party`** (implement it): the brain behind an actor. kamiroh calls
  `on_turn` with each incoming turn; the method's `&mut self` is the
  atomicity contract — one turn in, one atomic state change, at most one
  turn out, sent only after the method returns.
- **`Phone`** (hold it): the handle for initiating. Constructing one opens a
  conversation locally (nothing crosses the wire until the first turn);
  `open` starts an exchange, `send_turn` continues one, and alternation is
  enforced on both directions.

Turn bodies are opaque bytes: whatever envelope or encoding the app already
speaks rides through kamiroh untouched.

## Pattern: wrapping an existing request/response actor

If the app already has an actor (or any async component) with an ask-shaped
surface — "take a request value, return a response value" — the Party
wrapper is mechanical:

```rust
impl Party for MyActorBridge {
    async fn on_turn(&mut self, _from: &Address, turn: Turn) -> Option<Turn> {
        let request = turn.request()?;                  // Close → None: done
        let value = decode(&request.body);              // app's own encoding
        let answer = self.inner.ask(value).await;       // app's own actor
        Some(Turn::Close {
            response: Response { id: request.id, body: encode(&answer) },
        })
    }
}
```

Single-round exchanges (`Open` in, `Close` out) reproduce the app's existing
request/response idiom exactly. Multi-round exchanges (`Continue` turns) are
available whenever a dialogue is genuinely conversational. Errors the app
would return locally encode as error-shaped response bodies — kamiroh does
not interpret bodies.

Fire-and-forget notification patterns map as an `Open` answered by a trivial
`Close`: the sender gains delivery confidence it never had locally. There is
no fan-out primitive in v0; a broadcast to N remote subscribers is N
conversations.

## Pattern: proxy-then-promote (walking an actor tree onto the network)

Many apps keep their actors behind a supervisor: children created, messaged,
and closed through one funnel. Such a tree walks onto the network in two
optional steps — the second taken per child, later, with real usage in view.

**1. Proxy.** Install *one* kamiroh actor per app instance, backed by a Party
wrapper that translates single-round exchanges into the supervisor's existing
operations (create → id, forward(id, request) → response, close(id)). The
app's own message envelope rides in turn bodies. Zero changes to the app's
module; the whole tree becomes remotely reachable at once. Two costs, worth
accepting consciously: the allowlist can only grant access to the *whole
tree* (admitting an endpoint to the proxy is a privileged grant, in the same
sense as kamiroh's own harness actor — decision 6), and all remote traffic
serializes through the proxy's mailbox (if the supervisor already awaits its
children inline, this property exists locally; the network amplifies it).

**2. Promote.** When a specific child needs direct conversation, finer trust,
or parallelism, give it its own kamiroh actor: its id becomes an `ActorName`,
its wrapper asks it directly, its allowlist grants access to *it alone*, its
mailbox unblocks the funnel, and long multi-round exchanges with it stop
threading through the supervisor. Lifecycle coupling (create/close must now
bind/unbind the kamiroh actor) follows the same shape as kamiroh's harness
spawn/stop. The proxy remains for lifecycle and the unpromoted rest.

The steps compose — both are just parties behind actors — and nothing in
step 1 forecloses or prejudges step 2.

## Pattern: a third runtime (engine replacement)

The runtimes shipped here (`LocalRuntime`, `kamiroh-adapter-kameo`) host app
behavior inside an engine. An embedder whose own actor system must *host*
kamiroh actors (supervision trees, instrumentation, lifecycle idioms) can
write a driving adapter of the same shape, given three engine capabilities:
dynamic spawn/stop, a per-actor serialized mailbox (the atomicity substrate),
and tokio-compatible `Send` handler execution (decision 15).

The definition of done is already executable: the harness and turns
integration tests are the conformance suite. A third runtime is correct when
they pass against it. Absent a genuine need to host, wrapping via `Party`
(above) is smaller in every way.
