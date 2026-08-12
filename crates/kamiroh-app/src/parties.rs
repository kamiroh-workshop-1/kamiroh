//! Stock parties.
//!
//! Small [`Party`] implementations the system itself uses — the harness
//! spawns [`EchoParty`] behind new actors, and tests use both.

use kamiroh_domain::actor::Address;
use kamiroh_domain::protocol::TurnState;
use kamiroh_domain::vocabulary::{Response, Turn};
use kamiroh_ports::Party;

/// The simplest party: answers every request by echoing its body, never asks
/// anything of its own — so every exchange with it is a single round,
/// concluded by its `Close`.
#[derive(Debug, Default)]
pub struct EchoParty {
    state: TurnState,
}

impl EchoParty {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Party for EchoParty {
    async fn on_turn(&mut self, _from: &Address, turn: Turn) -> Option<Turn> {
        // Track the exchange with the same domain rulebook everyone uses;
        // an illegal turn gets no reply (the runtime has already validated,
        // so this is belt-and-braces).
        if self.state.on_incoming(&turn).is_err() {
            return None;
        }
        let request = turn.request()?.clone();
        let reply = Turn::Close {
            response: Response {
                id: request.id,
                body: request.body,
            },
        };
        self.state
            .on_outgoing(&reply)
            .expect("echo reply must be legal");
        Some(reply)
    }
}

/// A multi-round test party: answers each request and, while its counter is
/// above zero, poses a fresh request of its own (counting down) — producing
/// an exchange of `2n + 1` turns for an initial counter of `n`.
#[derive(Debug)]
pub struct CountdownParty {
    state: TurnState,
    remaining: u8,
    next_id: u8,
}

impl CountdownParty {
    pub fn new(rounds: u8) -> Self {
        Self {
            state: TurnState::Idle,
            remaining: rounds,
            next_id: 100,
        }
    }
}

impl Party for CountdownParty {
    async fn on_turn(&mut self, _from: &Address, turn: Turn) -> Option<Turn> {
        use kamiroh_domain::vocabulary::{Request, RequestId};
        if self.state.on_incoming(&turn).is_err() {
            return None;
        }
        let request = turn.request()?.clone();
        let response = Response {
            id: request.id,
            body: request.body,
        };
        let reply = if self.remaining > 0 {
            self.remaining -= 1;
            let id = RequestId([self.next_id; 16]);
            self.next_id = self.next_id.wrapping_add(1);
            Turn::Continue {
                response,
                request: Request {
                    id,
                    body: vec![self.remaining],
                },
            }
        } else {
            Turn::Close { response }
        };
        self.state
            .on_outgoing(&reply)
            .expect("countdown reply must be legal");
        Some(reply)
    }
}
