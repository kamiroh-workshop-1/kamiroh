//! Conversation state — the initiator-side sequential-exchange guard.
//!
//! A conversation runs **one exchange at a time** in v0 (`ARCHITECTURE.md`,
//! decision 10): a new exchange may not begin until the current one has
//! concluded. This type is where that rule lives.

use std::fmt;

use kamiroh_domain::actor::Address;
use kamiroh_domain::protocol::{completes, opens};
use kamiroh_domain::vocabulary::Message;

/// One side's view of a conversation with `peer`: which exchange, if any, is
/// currently in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    peer: Address,
    in_flight: Option<Message>,
}

impl Conversation {
    /// A conversation with no exchange under way. Conversations need no
    /// handshake — they begin implicitly with their first admitted delivery
    /// (`ARCHITECTURE.md`, decision 11).
    pub fn new(peer: Address) -> Self {
        Self {
            peer,
            in_flight: None,
        }
    }

    pub fn peer(&self) -> &Address {
        &self.peer
    }

    /// The opening message of the exchange in flight, if any.
    pub fn in_flight(&self) -> Option<&Message> {
        self.in_flight.as_ref()
    }

    /// Begin an exchange by recording its opening message. Refused if the
    /// message does not open an exchange, or one is already in flight.
    pub fn begin(&mut self, opening: &Message) -> Result<(), ExchangeError> {
        if !opens(opening) {
            return Err(ExchangeError::NotAnOpening);
        }
        if self.in_flight.is_some() {
            return Err(ExchangeError::AlreadyInFlight);
        }
        self.in_flight = Some(opening.clone());
        Ok(())
    }

    /// Conclude the exchange in flight with `reply`. Refused if nothing is in
    /// flight or the reply does not complete the exchange per its protocol.
    pub fn conclude(&mut self, reply: &Message) -> Result<(), ExchangeError> {
        let Some(opening) = &self.in_flight else {
            return Err(ExchangeError::NothingInFlight);
        };
        if !completes(opening, reply) {
            return Err(ExchangeError::WrongReply);
        }
        self.in_flight = None;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeError {
    /// The message is a reply kind; it cannot open an exchange.
    NotAnOpening,
    /// An exchange is already in flight — one at a time per conversation.
    AlreadyInFlight,
    /// No exchange is in flight to conclude.
    NothingInFlight,
    /// The reply does not complete the in-flight exchange per its protocol.
    WrongReply,
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ExchangeError::NotAnOpening => "message does not open an exchange",
            ExchangeError::AlreadyInFlight => "an exchange is already in flight",
            ExchangeError::NothingInFlight => "no exchange is in flight",
            ExchangeError::WrongReply => "reply does not complete the in-flight exchange",
        };
        f.write_str(s)
    }
}

impl std::error::Error for ExchangeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use kamiroh_domain::actor::ActorName;
    use kamiroh_domain::endpoint::EndpointId;
    use kamiroh_domain::hex::Hex;
    use kamiroh_domain::vocabulary::Harness;

    fn peer() -> Address {
        Address::new(
            EndpointId::new(Hex::new("bb").unwrap()),
            ActorName::new("bob").unwrap(),
        )
    }

    #[test]
    fn sequential_rule_is_enforced() {
        let mut conv = Conversation::new(peer());
        conv.begin(&Message::Harness(Harness::Ping)).unwrap();
        assert_eq!(
            conv.begin(&Message::Harness(Harness::Ping)),
            Err(ExchangeError::AlreadyInFlight)
        );
        conv.conclude(&Message::Harness(Harness::Pong)).unwrap();
        assert!(conv.in_flight().is_none());
        conv.begin(&Message::Harness(Harness::Ping)).unwrap();
    }

    #[test]
    fn replies_cannot_open() {
        let mut conv = Conversation::new(peer());
        assert_eq!(
            conv.begin(&Message::Harness(Harness::Pong)),
            Err(ExchangeError::NotAnOpening)
        );
    }

    #[test]
    fn wrong_reply_is_refused() {
        let mut conv = Conversation::new(peer());
        conv.begin(&Message::Harness(Harness::Ping)).unwrap();
        assert_eq!(
            conv.conclude(&Message::Harness(Harness::Stopped {
                name: ActorName::new("x").unwrap()
            })),
            Err(ExchangeError::WrongReply)
        );
    }

    #[test]
    fn concluding_nothing_is_refused() {
        let mut conv = Conversation::new(peer());
        assert_eq!(
            conv.conclude(&Message::Harness(Harness::Pong)),
            Err(ExchangeError::NothingInFlight)
        );
    }
}
