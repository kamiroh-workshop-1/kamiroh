//! Inbound delivery processing — the single choke point every delivery
//! passes through.
//!
//! Pairs the admission check (per delivery, deny by default) with the
//! protocol step the message calls for. The caller — ultimately the runtime
//! adapter — acts on the returned [`Inbound`]: hand the request to the party,
//! send the ready-made ack, drop denied traffic.

use kamiroh_domain::actor::Address;
use kamiroh_domain::allowlist::Allowlist;
use kamiroh_domain::vocabulary::{Ack, Harness, Message, Request, Turn};
use kamiroh_ports::Delivery;

use crate::admission::{Admission, admit};

/// What a processed delivery asks of the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    /// The origin endpoint is not admitted. Drop silently — an unadmitted
    /// sender learns nothing, not even that the actor exists.
    Denied,
    /// An admitted [`Request`]: hand `request` to the party behind
    /// `for_actor`, and send `ack` back to `reply_to` — the delivery
    /// acknowledgment of the request-ack protocol (not the party's answer;
    /// see ARCHITECTURE.md, decision 4).
    Request {
        request: Request,
        for_actor: Address,
        reply_to: Address,
        ack: Message,
    },
    /// An admitted [`Ack`]: a request of ours reached its destination actor.
    AckReceived(Ack),
    /// An admitted `harness` (lifecycle/test) message, for the runtime to
    /// interpret. Privileged — see ARCHITECTURE.md, decision 6.
    Harness { harness: Harness, reply_to: Address },
    /// An admitted [`Turn`]: hand it to the party behind `for_actor`. When
    /// the turn poses a request (`Open`/`Continue`), `ack` carries the
    /// delivery acknowledgment to send on handover — the fast receipt while
    /// the party thinks (decision 4). A `Close` gets no ack in v0 (its
    /// receipt is part of the deferred reliability work).
    Turn {
        turn: Turn,
        for_actor: Address,
        reply_to: Address,
        ack: Option<Message>,
    },
}

/// Process one delivery against the receiving actor's allowlist.
pub fn process(allowlist: &Allowlist, delivery: Delivery) -> Inbound {
    if admit(allowlist, &delivery) == Admission::Deny {
        return Inbound::Denied;
    }
    let Delivery { from, to, message } = delivery;
    match message {
        Message::Request(request) => {
            let ack = Message::Ack(Ack { id: request.id });
            Inbound::Request {
                request,
                for_actor: to,
                reply_to: from,
                ack,
            }
        }
        Message::Ack(ack) => Inbound::AckReceived(ack),
        Message::Harness(harness) => Inbound::Harness {
            harness,
            reply_to: from,
        },
        Message::Turn(turn) => {
            let ack = turn.request().map(|r| Message::Ack(Ack { id: r.id }));
            Inbound::Turn {
                turn,
                for_actor: to,
                reply_to: from,
                ack,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kamiroh_domain::actor::ActorName;
    use kamiroh_domain::endpoint::EndpointId;
    use kamiroh_domain::hex::Hex;
    use kamiroh_domain::vocabulary::RequestId;

    fn address(endpoint: &str, name: &str) -> Address {
        Address::new(
            EndpointId::new(Hex::new(endpoint).unwrap()),
            ActorName::new(name).unwrap(),
        )
    }

    fn allow(endpoint: &str) -> Allowlist {
        let mut list = Allowlist::empty();
        list.admit(EndpointId::new(Hex::new(endpoint).unwrap()));
        list
    }

    #[test]
    fn unadmitted_request_is_denied() {
        let delivery = Delivery {
            from: address("cc", "mallory"),
            to: address("bb", "bob"),
            message: Message::Request(Request {
                id: RequestId([1; 16]),
                body: vec![],
            }),
        };
        assert_eq!(process(&Allowlist::empty(), delivery), Inbound::Denied);
    }

    #[test]
    fn admitted_request_yields_ack_to_sender() {
        let id = RequestId([2; 16]);
        let delivery = Delivery {
            from: address("aa", "alice"),
            to: address("bb", "bob"),
            message: Message::Request(Request {
                id,
                body: b"hello".to_vec(),
            }),
        };
        match process(&allow("aa"), delivery) {
            Inbound::Request {
                request,
                for_actor,
                reply_to,
                ack,
            } => {
                assert_eq!(request.id, id);
                assert_eq!(for_actor, address("bb", "bob"));
                assert_eq!(reply_to, address("aa", "alice"));
                assert_eq!(ack, Message::Ack(Ack { id }));
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn admitted_ack_is_surfaced() {
        let id = RequestId([3; 16]);
        let delivery = Delivery {
            from: address("aa", "alice"),
            to: address("bb", "bob"),
            message: Message::Ack(Ack { id }),
        };
        assert_eq!(
            process(&allow("aa"), delivery),
            Inbound::AckReceived(Ack { id })
        );
    }
}
