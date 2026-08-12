//! The Phone — the driving handle an embedding app holds to converse
//! (`ARCHITECTURE.md`, decision 16).
//!
//! Opening a conversation is purely local (decision 11): constructing a Phone
//! allocates the conversation's turn state on this side; nothing crosses the
//! wire until the first turn is sent. The Phone enforces the alternation
//! rules — a turn can only be sent when it is legally ours to send, and
//! incoming turns are validated before the app sees their content as part of
//! a live exchange.
//!
//! Parties replying from inside [`Party::on_turn`](kamiroh_ports::Party) do
//! not need a Phone — the runtime sends their returned turn. The Phone is for
//! *initiating*: the app-side surface that opens exchanges.

use std::fmt;

use kamiroh_domain::actor::Address;
use kamiroh_domain::protocol::{TurnError, TurnProgress, TurnState};
use kamiroh_domain::vocabulary::{Message, Request, Turn};
use kamiroh_ports::Transport;

/// A live handle on one conversation: this actor ↔ `peer`.
#[derive(Debug)]
pub struct Phone<T: Transport> {
    self_address: Address,
    peer: Address,
    transport: T,
    state: TurnState,
}

impl<T: Transport> Phone<T> {
    /// Open a conversation with `peer` — a local act; the wire is first
    /// touched by [`Phone::open`].
    pub fn converse(self_address: Address, peer: Address, transport: T) -> Self {
        Self {
            self_address,
            peer,
            transport,
            state: TurnState::Idle,
        }
    }

    pub fn peer(&self) -> &Address {
        &self.peer
    }

    pub fn state(&self) -> TurnState {
        self.state
    }

    /// Open an exchange: send the opening turn posing `request`.
    pub async fn open(&mut self, request: Request) -> Result<(), PhoneError> {
        self.send_turn(Turn::Open { request }).await.map(|_| ())
    }

    /// Send any turn, enforcing alternation. Returns whether the exchange
    /// continues or (on a `Close`) is concluded.
    pub async fn send_turn(&mut self, turn: Turn) -> Result<TurnProgress, PhoneError> {
        let progress = self.state.on_outgoing(&turn)?;
        self.transport
            .send(&self.self_address, &self.peer, Message::Turn(turn))
            .await
            .map_err(|e| PhoneError::Transport(e.to_string()))?;
        Ok(progress)
    }

    /// Feed an incoming turn from this conversation's peer through the
    /// alternation rules. The caller (runtime or app pump) does this before
    /// treating the turn's content as part of the live exchange.
    pub fn on_incoming(&mut self, turn: &Turn) -> Result<TurnProgress, PhoneError> {
        Ok(self.state.on_incoming(turn)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneError {
    /// The turn violates alternation or answers the wrong request.
    Turn(TurnError),
    /// The transport refused the send.
    Transport(String),
}

impl From<TurnError> for PhoneError {
    fn from(e: TurnError) -> Self {
        PhoneError::Turn(e)
    }
}

impl fmt::Display for PhoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PhoneError::Turn(e) => write!(f, "turn refused: {e}"),
            PhoneError::Transport(e) => write!(f, "transport failed: {e}"),
        }
    }
}

impl std::error::Error for PhoneError {}
