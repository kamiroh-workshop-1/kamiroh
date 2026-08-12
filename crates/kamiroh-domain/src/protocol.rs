//! Protocols — named, legal sequences of vocabulary messages.
//!
//! A protocol is the rulebook, not an instance: it defines which messages
//! *open* an exchange, and which reply *completes* one. An **exchange** is one
//! complete run of a protocol within a conversation — however many round
//! trips the protocol defines (request-ack is the degenerate two-message
//! case). Each party to a protocol is opaque: an agent or an embedding
//! application, on one side or both.

use crate::vocabulary::{Harness, Message};

/// The protocols defined in v0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProtocolId {
    /// One [`Request`](crate::vocabulary::Request), one
    /// [`Ack`](crate::vocabulary::Ack). The first and simplest protocol.
    RequestAck,
    /// The lifecycle/test protocol (spawn / stop / ping). Privileged.
    Harness,
}

/// Which protocol a message belongs to.
pub fn protocol_of(message: &Message) -> ProtocolId {
    match message {
        Message::Request(_) | Message::Ack(_) => ProtocolId::RequestAck,
        Message::Harness(_) => ProtocolId::Harness,
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
        _ => false,
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
