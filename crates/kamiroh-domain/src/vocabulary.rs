//! Vocabulary v0 — the closed set of message kinds actors may exchange.
//!
//! Agnostic to the kind of agent (or non-agent) behind either end. Wire
//! encoding is an adapter concern; these are domain values only.

use crate::actor::ActorName;

/// Correlates an [`Ack`] (and, later, a `Response`) with its [`Request`].
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub [u8; 16]);

/// A payload addressed to the party behind an actor.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
/// Its exchanges are command/reply pairs: `Spawn → Spawned`, `Stop → Stopped`,
/// `Ping → Pong`, with [`Harness::Failed`] as the error reply to any command.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Harness {
    /// Command: spawn a named actor at the receiving endpoint.
    Spawn { name: ActorName },
    /// Reply: the named actor was spawned.
    Spawned { name: ActorName },
    /// Command: stop a previously spawned actor.
    Stop { name: ActorName },
    /// Reply: the named actor was stopped.
    Stopped { name: ActorName },
    /// Command: liveness probe.
    Ping,
    /// Reply to [`Harness::Ping`].
    Pong,
    /// Reply: the command could not be carried out.
    Failed { reason: String },
}

/// The party's actual answer to a [`Request`] — distinct from [`Ack`], which
/// is only the delivery receipt (`ARCHITECTURE.md`, decision 4).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The request this answers.
    pub id: RequestId,
    /// Opaque to the vocabulary in v0.
    pub body: Vec<u8>,
}

/// One unit of party-level messaging in the `turns` protocol: "here is my
/// answer to what you asked; here is what I now ask" — with either half
/// absent only at the exchange's boundaries. The enum encodes that a turn is
/// never empty (`ARCHITECTURE.md`, decision 17).
///
/// An exchange is an alternating sequence of turns: opened by [`Turn::Open`]
/// (a request, nothing yet to answer), continued by [`Turn::Continue`]
/// (answer + new request), concluded by [`Turn::Close`] (answer, nothing
/// further asked). One incoming turn = one atomic party state change = at
/// most one outgoing turn, emitted only after the state settles.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
    /// Opens an exchange: a request with no response half.
    Open { request: Request },
    /// Continues an exchange: answers the outstanding request and poses the
    /// next one.
    Continue {
        response: Response,
        request: Request,
    },
    /// Concludes an exchange: answers the outstanding request, asks nothing.
    Close { response: Response },
}

impl Turn {
    /// The response half, if present.
    pub fn response(&self) -> Option<&Response> {
        match self {
            Turn::Open { .. } => None,
            Turn::Continue { response, .. } | Turn::Close { response } => Some(response),
        }
    }

    /// The request half, if present — the new outstanding request after this
    /// turn.
    pub fn request(&self) -> Option<&Request> {
        match self {
            Turn::Open { request } | Turn::Continue { request, .. } => Some(request),
            Turn::Close { .. } => None,
        }
    }
}

/// Everything an actor may say. Closed in v0 (`ARCHITECTURE.md`, decision 5).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Request(Request),
    Ack(Ack),
    Harness(Harness),
    Turn(Turn),
}
