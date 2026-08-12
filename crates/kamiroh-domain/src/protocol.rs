//! Protocols — named, legal sequences of vocabulary messages.
//!
//! A protocol is the rulebook, not an instance: it defines which messages
//! *open* an exchange, and which reply *completes* one. An **exchange** is one
//! complete run of a protocol within a conversation — however many round
//! trips the protocol defines (request-ack is the degenerate two-message
//! case). Each party to a protocol is opaque: an agent or an embedding
//! application, on one side or both.

use std::fmt;

use crate::vocabulary::{Harness, Message, RequestId, Turn};

/// The protocols defined in v0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProtocolId {
    /// One [`Request`](crate::vocabulary::Request), one
    /// [`Ack`](crate::vocabulary::Ack). The first and simplest protocol.
    RequestAck,
    /// The lifecycle/test protocol (spawn / stop / ping). Privileged.
    Harness,
    /// Party-level conversation in alternating [`Turn`]s
    /// (`ARCHITECTURE.md`, decision 17).
    Turns,
}

/// Which protocol a message belongs to.
pub fn protocol_of(message: &Message) -> ProtocolId {
    match message {
        Message::Request(_) | Message::Ack(_) => ProtocolId::RequestAck,
        Message::Harness(_) => ProtocolId::Harness,
        Message::Turn(_) => ProtocolId::Turns,
    }
}

/// Does this message open an exchange?
///
/// Opening messages start a run of their protocol; everything else is a reply
/// within an exchange already under way.
pub fn opens(message: &Message) -> bool {
    match message {
        Message::Request(_) => true,
        Message::Ack(_) => false,
        Message::Harness(h) => matches!(
            h,
            Harness::Spawn { .. } | Harness::Stop { .. } | Harness::Ping
        ),
        Message::Turn(t) => matches!(t, Turn::Open { .. }),
    }
}

/// Does `reply` complete the exchange opened by `opening`?
///
/// v0 protocols all complete in a single round trip; longer protocols will
/// extend this into a fuller state machine.
pub fn completes(opening: &Message, reply: &Message) -> bool {
    match (opening, reply) {
        (Message::Request(request), Message::Ack(ack)) => ack.id == request.id,
        (Message::Harness(command), Message::Harness(reply)) => match (command, reply) {
            (Harness::Ping, Harness::Pong) => true,
            (Harness::Spawn { name }, Harness::Spawned { name: spawned }) => name == spawned,
            (Harness::Stop { name }, Harness::Stopped { name: stopped }) => name == stopped,
            (
                Harness::Spawn { .. } | Harness::Stop { .. } | Harness::Ping,
                Harness::Failed { .. },
            ) => true,
            _ => false,
        },
        // The single-round turn exchange; multi-round exchanges are tracked
        // by [`TurnState`], the authoritative rulebook for `turns`.
        (Message::Turn(Turn::Open { request }), Message::Turn(Turn::Close { response })) => {
            response.id == request.id
        }
        _ => false,
    }
}

/// One side's state machine for a `turns` exchange — the authoritative
/// alternation rulebook (`ARCHITECTURE.md`, decision 17).
///
/// Both parties hold one; the machine is symmetric, distinguished only by
/// which methods fire. Every legal exchange walks: `Idle` →
/// (open) → alternating `AwaitingTheirTurn` / `OweThem` → (close) → `Idle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnState {
    /// No exchange under way.
    #[default]
    Idle,
    /// We spoke last; their turn. `outstanding` is the request we posed.
    AwaitingTheirTurn { outstanding: RequestId },
    /// They spoke last; our turn. `outstanding` is the request we must answer.
    OweThem { outstanding: RequestId },
}

/// Where the exchange stands after a legal turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnProgress {
    /// The exchange continues; the other side now owes a turn.
    Continuing,
    /// The closing turn: the exchange is concluded, state is `Idle` again.
    Concluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnError {
    /// It is the other side's move; we may not send.
    NotOurMove,
    /// It is our move; an incoming turn is illegal.
    NotTheirMove,
    /// A response half does not answer the outstanding request.
    WrongResponse,
    /// A `Continue`/`Close` with no exchange under way.
    NoExchange,
    /// An `Open` while a request is outstanding — it must be answered first.
    MustAnswerFirst,
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TurnError::NotOurMove => "not our move: awaiting the other side's turn",
            TurnError::NotTheirMove => "not their move: we owe the next turn",
            TurnError::WrongResponse => "response does not answer the outstanding request",
            TurnError::NoExchange => "no exchange is under way",
            TurnError::MustAnswerFirst => "an outstanding request must be answered first",
        };
        f.write_str(s)
    }
}

impl std::error::Error for TurnError {}

impl TurnState {
    /// Validate and apply a turn we are about to send.
    pub fn on_outgoing(&mut self, turn: &Turn) -> Result<TurnProgress, TurnError> {
        match (*self, turn) {
            (TurnState::Idle, Turn::Open { request }) => {
                *self = TurnState::AwaitingTheirTurn {
                    outstanding: request.id,
                };
                Ok(TurnProgress::Continuing)
            }
            (TurnState::Idle, _) => Err(TurnError::NoExchange),
            (TurnState::OweThem { outstanding }, Turn::Continue { response, request }) => {
                if response.id != outstanding {
                    return Err(TurnError::WrongResponse);
                }
                *self = TurnState::AwaitingTheirTurn {
                    outstanding: request.id,
                };
                Ok(TurnProgress::Continuing)
            }
            (TurnState::OweThem { outstanding }, Turn::Close { response }) => {
                if response.id != outstanding {
                    return Err(TurnError::WrongResponse);
                }
                *self = TurnState::Idle;
                Ok(TurnProgress::Concluded)
            }
            (TurnState::OweThem { .. }, Turn::Open { .. }) => Err(TurnError::MustAnswerFirst),
            (TurnState::AwaitingTheirTurn { .. }, _) => Err(TurnError::NotOurMove),
        }
    }

    /// Validate and apply a turn arriving from the other side.
    pub fn on_incoming(&mut self, turn: &Turn) -> Result<TurnProgress, TurnError> {
        match (*self, turn) {
            (TurnState::Idle, Turn::Open { request }) => {
                *self = TurnState::OweThem {
                    outstanding: request.id,
                };
                Ok(TurnProgress::Continuing)
            }
            (TurnState::Idle, _) => Err(TurnError::NoExchange),
            (
                TurnState::AwaitingTheirTurn { outstanding },
                Turn::Continue { response, request },
            ) => {
                if response.id != outstanding {
                    return Err(TurnError::WrongResponse);
                }
                *self = TurnState::OweThem {
                    outstanding: request.id,
                };
                Ok(TurnProgress::Continuing)
            }
            (TurnState::AwaitingTheirTurn { outstanding }, Turn::Close { response }) => {
                if response.id != outstanding {
                    return Err(TurnError::WrongResponse);
                }
                *self = TurnState::Idle;
                Ok(TurnProgress::Concluded)
            }
            (TurnState::AwaitingTheirTurn { .. }, Turn::Open { .. }) => {
                Err(TurnError::MustAnswerFirst)
            }
            (TurnState::OweThem { .. }, _) => Err(TurnError::NotTheirMove),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::ActorName;
    use crate::vocabulary::{Ack, Request, RequestId};

    fn name(s: &str) -> ActorName {
        ActorName::new(s).unwrap()
    }

    #[test]
    fn openings_and_replies_are_disjoint() {
        let request = Message::Request(Request {
            id: RequestId([1; 16]),
            body: vec![],
        });
        assert!(opens(&request));
        assert!(!opens(&Message::Ack(Ack {
            id: RequestId([1; 16])
        })));
        assert!(opens(&Message::Harness(Harness::Ping)));
        assert!(!opens(&Message::Harness(Harness::Pong)));
        assert!(!opens(&Message::Harness(Harness::Failed {
            reason: "x".into()
        })));
    }

    #[test]
    fn request_completed_by_matching_ack_only() {
        let opening = Message::Request(Request {
            id: RequestId([1; 16]),
            body: vec![],
        });
        let right = Message::Ack(Ack {
            id: RequestId([1; 16]),
        });
        let wrong = Message::Ack(Ack {
            id: RequestId([2; 16]),
        });
        assert!(completes(&opening, &right));
        assert!(!completes(&opening, &wrong));
    }

    #[test]
    fn a_multi_round_exchange_walks_both_machines_to_idle() {
        use crate::vocabulary::{Request, RequestId, Response, Turn};
        let r = |n: u8| Request {
            id: RequestId([n; 16]),
            body: vec![n],
        };
        let resp = |n: u8| Response {
            id: RequestId([n; 16]),
            body: vec![n],
        };

        let mut alice = TurnState::default();
        let mut bob = TurnState::default();

        // alice opens with request 1.
        let t1 = Turn::Open { request: r(1) };
        assert_eq!(alice.on_outgoing(&t1), Ok(TurnProgress::Continuing));
        assert_eq!(bob.on_incoming(&t1), Ok(TurnProgress::Continuing));

        // bob answers 1 and poses 2.
        let t2 = Turn::Continue {
            response: resp(1),
            request: r(2),
        };
        assert_eq!(bob.on_outgoing(&t2), Ok(TurnProgress::Continuing));
        assert_eq!(alice.on_incoming(&t2), Ok(TurnProgress::Continuing));

        // alice answers 2, asks nothing: exchange concluded on both sides.
        let t3 = Turn::Close { response: resp(2) };
        assert_eq!(alice.on_outgoing(&t3), Ok(TurnProgress::Concluded));
        assert_eq!(bob.on_incoming(&t3), Ok(TurnProgress::Concluded));
        assert_eq!(alice, TurnState::Idle);
        assert_eq!(bob, TurnState::Idle);
    }

    #[test]
    fn alternation_violations_and_wrong_ids_are_refused() {
        use crate::vocabulary::{Request, RequestId, Response, Turn};
        let r1 = Request {
            id: RequestId([1; 16]),
            body: vec![],
        };
        let open = Turn::Open { request: r1 };

        let mut s = TurnState::default();
        s.on_outgoing(&open).unwrap();
        // We just spoke; sending again is not our move.
        assert_eq!(s.on_outgoing(&open), Err(TurnError::NotOurMove));
        // Their close must answer request 1, not something else.
        assert_eq!(
            s.on_incoming(&Turn::Close {
                response: Response {
                    id: RequestId([9; 16]),
                    body: vec![]
                }
            }),
            Err(TurnError::WrongResponse)
        );
        // A second Open from them while 1 is outstanding is illegal.
        assert_eq!(
            s.on_incoming(&Turn::Open {
                request: Request {
                    id: RequestId([2; 16]),
                    body: vec![]
                }
            }),
            Err(TurnError::MustAnswerFirst)
        );
        // From Idle, a bare Close is meaningless.
        assert_eq!(
            TurnState::default().on_incoming(&Turn::Close {
                response: Response {
                    id: RequestId([1; 16]),
                    body: vec![]
                }
            }),
            Err(TurnError::NoExchange)
        );
    }

    #[test]
    fn harness_commands_complete_with_matching_replies_or_failed() {
        let spawn = Message::Harness(Harness::Spawn { name: name("echo") });
        assert!(completes(
            &spawn,
            &Message::Harness(Harness::Spawned { name: name("echo") })
        ));
        assert!(!completes(
            &spawn,
            &Message::Harness(Harness::Spawned {
                name: name("other")
            })
        ));
        assert!(completes(
            &spawn,
            &Message::Harness(Harness::Failed {
                reason: "name in use".into()
            })
        ));
        assert!(!completes(
            &Message::Harness(Harness::Ping),
            &Message::Harness(Harness::Stopped { name: name("echo") })
        ));
    }
}
