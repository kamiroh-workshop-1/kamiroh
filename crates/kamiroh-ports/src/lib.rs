//! kamiroh's ports.
//!
//! Traits defined by the core: driven ports are implemented by adapters
//! (`kamiroh-adapter-*`), driving ports are consumed by embedding applications
//! and agent harnesses. Adapters depend on `kamiroh-domain` + this crate only —
//! never on the application layer — so the hexagon's dependency arrows are
//! compiler-enforced.

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
    ///
    /// Implementations' futures must be `Send`: these ports are crossed by
    /// multi-threaded runtimes by design (ARCHITECTURE.md, decision 15).
    fn send(
        &mut self,
        from: &Address,
        to: &Address,
        message: Message,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

/// Driving port: the inbound surface handed to an embedding application or an
/// agent's harness — messages arriving for its dedicated actor.
pub trait Inbox {
    /// The next delivery, or `None` when the conversation source is closed.
    ///
    /// Implementations' futures must be `Send`: these ports are crossed by
    /// multi-threaded runtimes by design (ARCHITECTURE.md, decision 15).
    fn next(&mut self) -> impl std::future::Future<Output = Option<Delivery>> + Send;
}

/// Driven port on the app side of the hexagon: **the party behind an actor**
/// (`ARCHITECTURE.md`, decision 16). The embedding application implements
/// this, one per actor; kamiroh drives it — push, not pull.
///
/// The signature *is* the atomicity contract (decision 17): one incoming turn
/// → one atomic state change (guarded by `&mut self` and the runtime's
/// per-actor serialization) → at most one outgoing turn, emitted by the
/// runtime only after this method returns, i.e. after the state has settled.
///
/// Contract for the return value, enforced by the runtime's `TurnState`:
/// - Incoming `Open`/`Continue` (a request is posed): return `Some(turn)`
///   whose response half answers it — `Continue` to keep the exchange going,
///   `Close` to conclude it.
/// - Incoming `Close` (nothing asked): return `None`; the exchange is over.
pub trait Party {
    fn on_turn(
        &mut self,
        from: &Address,
        turn: kamiroh_domain::vocabulary::Turn,
    ) -> impl std::future::Future<Output = Option<kamiroh_domain::vocabulary::Turn>> + Send;
}

/// Object-safe form of [`Party`], for runtimes hosting heterogeneous parties.
/// Blanket-implemented; implement [`Party`], not this.
pub trait DynParty: Send {
    fn on_turn_boxed<'a>(
        &'a mut self,
        from: &'a Address,
        turn: kamiroh_domain::vocabulary::Turn,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<kamiroh_domain::vocabulary::Turn>> + Send + 'a>,
    >;
}

impl<P: Party + Send> DynParty for P {
    fn on_turn_boxed<'a>(
        &'a mut self,
        from: &'a Address,
        turn: kamiroh_domain::vocabulary::Turn,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<kamiroh_domain::vocabulary::Turn>> + Send + 'a>,
    > {
        Box::pin(self.on_turn(from, turn))
    }
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
