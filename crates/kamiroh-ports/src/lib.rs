//! kamiroh's ports.
//!
//! Traits defined by the core: driven ports are implemented by adapters
//! (`kamiroh-adapter-*`), driving ports are consumed by embedding applications
//! and agent harnesses. Adapters depend on `kamiroh-domain` + this crate only —
//! never on the application layer — so the hexagon's dependency arrows are
//! compiler-enforced.
#![allow(async_fn_in_trait)] // spike scope: single-crate consumers, no dyn use yet

use kamiroh_domain::actor::Address;
use kamiroh_domain::vocabulary::Message;

/// An inbound delivery as witnessed by the transport.
///
/// `from.endpoint` is transport-proven; `from.name` is claimed by the remote
/// runtime. `to` is the local actor the message is addressed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub from: Address,
    pub to: Address,
    pub message: Message,
}

/// Driven port: carries vocabulary messages between actors.
///
/// Implemented by `kamiroh-adapter-iroh` for real conversations (short- or
/// long-lived) and by `kamiroh-adapter-memory` for in-process tests.
pub trait Transport {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Open (or reuse) a conversation with the actor at `to` and send
    /// `message` as `from`.
    async fn send(
        &mut self,
        from: &Address,
        to: &Address,
        message: Message,
    ) -> Result<(), Self::Error>;
}

/// Driving port: the inbound surface handed to an embedding application or an
/// agent's harness — messages arriving for its dedicated actor.
pub trait Inbox {
    /// The next delivery, or `None` when the conversation source is closed.
    async fn next(&mut self) -> Option<Delivery>;
}

/// Driven port: bind a local actor's [`Address`] so the transport routes
/// deliveries to it (`ARCHITECTURE.md`, decision 12).
///
/// Dropping the returned [`Inbox`] unbinds the address. The memory net
/// implements binding as registration; the Iroh adapter will implement it as
/// routing inside the endpoint.
pub trait Registry {
    type Inbox: Inbox;
    type Error: std::error::Error + Send + Sync + 'static;

    fn bind(&mut self, address: &Address) -> Result<Self::Inbox, Self::Error>;
}
