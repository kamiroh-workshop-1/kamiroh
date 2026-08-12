//! In-process transport adapter.
//!
//! Implements the [`Transport`] and [`Inbox`] ports over in-memory mailboxes,
//! so the application layer can be exercised in tests with no network
//! involved. Zero dependencies beyond the core: waiting is implemented with
//! std wakers, so any executor can drive it — including the minimal
//! [`testing::block_on`] this crate ships for tests.
//!
//! ## Trust caveat — test affordance
//!
//! [`MemoryTransport::send`] accepts the sender's `from` address as given:
//! callers can claim any origin, which is exactly what makes allowlist-denial
//! tests easy to write. Real transports must do the opposite — the receiving
//! adapter derives `Delivery::from.endpoint` from the *authenticated
//! connection*, never from the sender's claim. `kamiroh-adapter-iroh` will get
//! this from Iroh's connection handshake.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use kamiroh_domain::actor::Address;
use kamiroh_domain::vocabulary::Message;
use kamiroh_ports::{Delivery, Inbox, Registry, Transport};

pub mod testing;

#[derive(Debug, Default)]
struct Mailbox {
    queue: VecDeque<Delivery>,
    waker: Option<Waker>,
}

#[derive(Debug, Default)]
struct Shared {
    mailboxes: HashMap<Address, Mailbox>,
}

/// An in-process "network": a registry of actor mailboxes.
///
/// Clone handles freely; they all point at the same network.
#[derive(Clone, Default)]
pub struct MemoryNet {
    shared: Arc<Mutex<Shared>>,
}

impl MemoryNet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an actor at `address`, returning the [`Inbox`] its messages
    /// arrive on. The mailbox lives until the returned inbox is dropped.
    pub fn register(&self, address: Address) -> Result<MemoryInbox, RegisterError> {
        let mut shared = self.shared.lock().expect("memory net poisoned");
        if shared.mailboxes.contains_key(&address) {
            return Err(RegisterError::AddressInUse);
        }
        shared.mailboxes.insert(address.clone(), Mailbox::default());
        Ok(MemoryInbox {
            address,
            shared: Arc::clone(&self.shared),
        })
    }

    /// A [`Transport`] handle onto this network.
    pub fn transport(&self) -> MemoryTransport {
        MemoryTransport {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Registry for MemoryNet {
    type Inbox = MemoryInbox;
    type Error = RegisterError;

    fn bind(&mut self, address: &Address) -> Result<Self::Inbox, Self::Error> {
        self.register(address.clone())
    }
}

/// [`Transport`] implementation over [`MemoryNet`].
#[derive(Clone)]
pub struct MemoryTransport {
    shared: Arc<Mutex<Shared>>,
}

impl Transport for MemoryTransport {
    type Error = MemoryTransportError;

    async fn send(
        &mut self,
        from: &Address,
        to: &Address,
        message: Message,
    ) -> Result<(), Self::Error> {
        let mut shared = self.shared.lock().expect("memory net poisoned");
        let mailbox = shared
            .mailboxes
            .get_mut(to)
            .ok_or(MemoryTransportError::UnknownAddress)?;
        mailbox.queue.push_back(Delivery {
            from: from.clone(),
            to: to.clone(),
            message,
        });
        if let Some(waker) = mailbox.waker.take() {
            waker.wake();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTransportError {
    /// No actor is registered at the target address.
    UnknownAddress,
}

impl fmt::Display for MemoryTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryTransportError::UnknownAddress => {
                f.write_str("no actor is registered at the target address")
            }
        }
    }
}

impl std::error::Error for MemoryTransportError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterError {
    /// An actor with this address is already registered.
    AddressInUse,
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegisterError::AddressInUse => {
                f.write_str("an actor with this address is already registered")
            }
        }
    }
}

impl std::error::Error for RegisterError {}

/// [`Inbox`] implementation over [`MemoryNet`]. Dropping it unregisters the
/// actor; subsequent sends to its address fail with
/// [`MemoryTransportError::UnknownAddress`].
#[derive(Debug)]
pub struct MemoryInbox {
    address: Address,
    shared: Arc<Mutex<Shared>>,
}

impl Inbox for MemoryInbox {
    async fn next(&mut self) -> Option<Delivery> {
        NextDelivery { inbox: self }.await
    }
}

impl Drop for MemoryInbox {
    fn drop(&mut self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.mailboxes.remove(&self.address);
        }
    }
}

struct NextDelivery<'a> {
    inbox: &'a MemoryInbox,
}

impl Future for NextDelivery<'_> {
    type Output = Option<Delivery>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared = self.inbox.shared.lock().expect("memory net poisoned");
        let Some(mailbox) = shared.mailboxes.get_mut(&self.inbox.address) else {
            return Poll::Ready(None);
        };
        match mailbox.queue.pop_front() {
            Some(delivery) => Poll::Ready(Some(delivery)),
            None => {
                mailbox.waker = Some(cx.waker().clone());
                Poll::Pending
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
    use kamiroh_domain::vocabulary::{Harness, Message};

    use crate::testing::block_on;

    fn address(endpoint: &str, name: &str) -> Address {
        Address::new(
            EndpointId::new(Hex::new(endpoint).unwrap()),
            ActorName::new(name).unwrap(),
        )
    }

    #[test]
    fn send_to_unknown_address_errors() {
        let net = MemoryNet::new();
        let mut t = net.transport();
        let err = block_on(t.send(
            &address("aa", "alice"),
            &address("bb", "bob"),
            Message::Harness(Harness::Ping),
        ));
        assert_eq!(err, Err(MemoryTransportError::UnknownAddress));
    }

    #[test]
    fn deliveries_arrive_in_order() {
        let net = MemoryNet::new();
        let alice = address("aa", "alice");
        let bob = address("bb", "bob");
        let mut inbox = net.register(bob.clone()).unwrap();
        let mut t = net.transport();
        block_on(async {
            t.send(&alice, &bob, Message::Harness(Harness::Ping))
                .await
                .unwrap();
            t.send(&alice, &bob, Message::Harness(Harness::Pong))
                .await
                .unwrap();
            let first = inbox.next().await.unwrap();
            let second = inbox.next().await.unwrap();
            assert_eq!(first.message, Message::Harness(Harness::Ping));
            assert_eq!(second.message, Message::Harness(Harness::Pong));
            assert_eq!(first.from, alice);
            assert_eq!(first.to, bob);
        });
    }

    #[test]
    fn duplicate_registration_is_refused() {
        let net = MemoryNet::new();
        let bob = address("bb", "bob");
        let _inbox = net.register(bob.clone()).unwrap();
        assert_eq!(net.register(bob).unwrap_err(), RegisterError::AddressInUse);
    }

    #[test]
    fn dropping_inbox_unregisters() {
        let net = MemoryNet::new();
        let bob = address("bb", "bob");
        let inbox = net.register(bob.clone()).unwrap();
        drop(inbox);
        let mut t = net.transport();
        let err = block_on(t.send(
            &address("aa", "alice"),
            &bob,
            Message::Harness(Harness::Ping),
        ));
        assert_eq!(err, Err(MemoryTransportError::UnknownAddress));
    }

    #[test]
    fn pending_receiver_is_woken_by_a_send() {
        let net = MemoryNet::new();
        let alice = address("aa", "alice");
        let bob = address("bb", "bob");
        let mut inbox = net.register(bob.clone()).unwrap();
        let net2 = net.clone();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut t = net2.transport();
            block_on(t.send(&alice, &bob, Message::Harness(Harness::Ping))).unwrap();
        });
        let delivery = block_on(inbox.next()).unwrap();
        assert_eq!(delivery.message, Message::Harness(Harness::Ping));
        sender.join().unwrap();
    }
}
