//! Vocabulary v0 — the closed set of message kinds actors may exchange.
//!
//! Agnostic to the kind of agent (or non-agent) behind either end. Wire
//! encoding is an adapter concern; these are domain values only.

use crate::actor::ActorName;

/// Correlates an [`Ack`] (and, later, a `Response`) with its [`Request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub [u8; 16]);

/// A payload addressed to the party behind an actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: RequestId,
    /// Opaque to the vocabulary in v0.
    pub body: Vec<u8>,
}

/// Delivery acknowledgment from the remote **actor**: "the request reached
/// the dedicated actor and was handed over."
///
/// Deliberately distinct from any future `Response` (the party's actual
/// answer), so response semantics can arrive later without remodeling
/// (`ARCHITECTURE.md`, decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ack {
    pub id: RequestId,
}

/// The lifecycle/test vocabulary of the `harness` protocol.
///
/// Exists so integration tests can orchestrate both ends of a real
/// conversation using the system's own machinery. Admitting an endpoint to
/// this protocol is a privileged grant; the general agent-control vocabulary
/// is deliberately deferred (`ARCHITECTURE.md`, decision 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Harness {
    /// Spawn a named actor backed by a trivial echo-style agent.
    Spawn {
        name: ActorName,
    },
    /// Stop a previously spawned actor.
    Stop {
        name: ActorName,
    },
    Ping,
    Pong,
}

/// Everything an actor may say. Closed in v0 (`ARCHITECTURE.md`, decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Request(Request),
    Ack(Ack),
    Harness(Harness),
}
