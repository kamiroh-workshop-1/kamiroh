//! Kameo runtime adapter.
//!
//! Animates domain actors as Kameo actors — the engine-for-engine replacement
//! for [`kamiroh_app::runtime::LocalRuntime`]. This is a *driving* adapter
//! (`ARCHITECTURE.md`, decision 13): it hosts the app layer's behavior — one
//! Kameo actor per domain actor, each fed by a pump task draining its
//! transport [`Inbox`], each delivery routed through
//! [`inbound::process`](kamiroh_app::inbound::process).
//!
//! The toy `LocalRuntime` stays in the tree as the reference implementation;
//! this adapter reproduces its observable behavior with real concurrency —
//! actors run autonomously, no manual `step()`.
//!
//! ## Version assumptions — read me on the first local build pass
//!
//! Written against `kameo = "0.17"` and `tokio = "1"` **without compiling**
//! (the cloud sandbox cannot reach crates.io). Points most likely to need
//! adjustment against the actual latest kameo:
//!
//! 1. The `Actor` derive vs manual impl (`Args`/`Error` associated types and
//!    `on_start` shape have churned across kameo versions).
//! 2. `kameo::spawn(actor)` vs `Actor::spawn(args)`.
//! 3. `actor_ref.tell(msg).await` vs `.tell(msg).send().await` and the
//!    `SendError` type.
//! 4. `Context` path and lifetime parameters in `Message::handle`.
//! 5. `ActorRef::kill` / `stop_gracefully` naming.
//!
//! Everything else — roster, pump, admission, harness semantics — is
//! kameo-agnostic and should not need touching.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kameo::actor::{ActorRef, Spawn};
use kameo::message::{Context, Message};

use kamiroh_app::inbound::{Inbound, process};
use kamiroh_app::parties::EchoParty;
use kamiroh_app::runtime::{ActorKind, RuntimeError};
use kamiroh_domain::actor::{ActorName, Address};
use kamiroh_domain::allowlist::Allowlist;
use kamiroh_domain::endpoint::EndpointId;
use kamiroh_domain::protocol::TurnState;
use kamiroh_domain::vocabulary::{Harness, Message as Vocab};
use kamiroh_ports::{DynParty, Inbox, Registry, Transport};

/// The Kameo-backed runtime for one endpoint. Cheap to clone; clones share
/// the roster.
///
/// Must be used inside a tokio runtime (pump tasks are `tokio::spawn`ed).
pub struct KameoRuntime<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    inner: Arc<Inner<T, R>>,
}

impl<T, R> Clone for KameoRuntime<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    endpoint: EndpointId,
    /// Template transport handle, cloned into each hosted actor.
    transport: T,
    registry: Mutex<R>,
    roster: Mutex<HashMap<ActorName, Entry<T, R>>>,
}

struct Entry<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    actor_ref: ActorRef<Host<T, R>>,
    pump: tokio::task::JoinHandle<()>,
}

impl<T, R> KameoRuntime<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    pub fn new(endpoint: EndpointId, transport: T, registry: R) -> Self {
        Self {
            inner: Arc::new(Inner {
                endpoint,
                transport,
                registry: Mutex::new(registry),
                roster: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn endpoint(&self) -> &EndpointId {
        &self.inner.endpoint
    }

    /// Install an actor: bind its address (Registry port), spawn its Kameo
    /// host, and start the pump task feeding deliveries from the transport
    /// inbox into the host's mailbox.
    pub fn install(
        &self,
        name: ActorName,
        allowlist: Allowlist,
        kind: ActorKind,
    ) -> Result<(), RuntimeError> {
        self.install_inner(name, allowlist, kind, None)
    }

    /// Install an actor with the party behind it (decision 16).
    pub fn install_party(
        &self,
        name: ActorName,
        allowlist: Allowlist,
        party: Box<dyn DynParty>,
    ) -> Result<(), RuntimeError> {
        self.install_inner(name, allowlist, ActorKind::Plain, Some(party))
    }

    fn install_inner(
        &self,
        name: ActorName,
        allowlist: Allowlist,
        kind: ActorKind,
        party: Option<Box<dyn DynParty>>,
    ) -> Result<(), RuntimeError> {
        let mut roster = self.inner.roster.lock().expect("roster poisoned");
        if roster.contains_key(&name) {
            return Err(RuntimeError::NameInUse);
        }
        let address = Address::new(self.inner.endpoint.clone(), name.clone());
        let mut inbox = self
            .inner
            .registry
            .lock()
            .expect("registry poisoned")
            .bind(&address)
            .map_err(|e| RuntimeError::Bind(e.to_string()))?;

        let host = Host {
            address,
            allowlist,
            kind,
            transport: self.inner.transport.clone(),
            runtime: self.clone(),
            party,
            turns: HashMap::new(),
        };
        let actor_ref = Host::spawn(host);

        let pump_ref = actor_ref.clone();
        let pump = tokio::spawn(async move {
            // The pump owns the transport inbox; when this task ends (or is
            // aborted by `stop`), the inbox drops, which unbinds the address
            // at the transport (Registry contract, decision 12).
            while let Some(delivery) = inbox.next().await {
                if pump_ref.tell(Deliver(delivery)).await.is_err() {
                    break; // host stopped
                }
            }
        });

        roster.insert(name, Entry { actor_ref, pump });
        Ok(())
    }

    /// Stop an actor: end its pump (unbinding its address) and stop its host.
    pub fn stop(&self, name: &ActorName) -> Result<(), RuntimeError> {
        let entry = {
            let mut roster = self.inner.roster.lock().expect("roster poisoned");
            roster.remove(name).ok_or(RuntimeError::UnknownActor)?
        };
        entry.pump.abort();
        entry.actor_ref.kill();
        Ok(())
    }
}

/// The Kameo actor hosting one domain actor's behavior.
struct Host<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    address: Address,
    allowlist: Allowlist,
    kind: ActorKind,
    transport: T,
    runtime: KameoRuntime<T, R>,
    /// The party behind this actor, if one is wired (decision 16).
    party: Option<Box<dyn DynParty>>,
    /// Per-conversation turn state, keyed by peer (decision 17).
    turns: HashMap<Address, TurnState>,
}

impl<T, R> kameo::Actor for Host<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    // Assumption point 1: adjust to the actual kameo Actor trait shape.
    type Args = Self;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        Ok(args)
    }
}

/// One inbound delivery, pumped from the transport inbox.
struct Deliver(kamiroh_ports::Delivery);

impl<T, R> Message<Deliver> for Host<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    type Reply = ();

    async fn handle(
        &mut self,
        Deliver(delivery): Deliver,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let self_address = self.address.clone();
        match process(&self.allowlist, delivery) {
            Inbound::Denied => {}
            Inbound::Request { reply_to, ack, .. } => {
                // Delivery acknowledged; no party is wired behind Plain
                // actors in v0. Send errors are dropped for now — the sender
                // times out rather than us crashing the host (spike scope).
                let _ = self.transport.send(&self_address, &reply_to, ack).await;
            }
            Inbound::AckReceived(_) => {}
            Inbound::Harness { harness, reply_to } => {
                let reply = if self.kind != ActorKind::Harness {
                    Some(Vocab::Harness(Harness::Failed {
                        reason: "not a harness actor".into(),
                    }))
                } else {
                    self.execute(harness, &reply_to)
                };
                if let Some(reply) = reply {
                    let _ = self.transport.send(&self_address, &reply_to, reply).await;
                }
            }
            Inbound::Turn {
                turn,
                for_actor: _,
                reply_to,
                ack,
            } => {
                // Validate against this conversation's alternation state;
                // illegal turns are dropped silently.
                let mut state = self.turns.get(&reply_to).copied().unwrap_or_default();
                if state.on_incoming(&turn).is_err() {
                    return;
                }
                self.turns.insert(reply_to.clone(), state);
                // Ack on handover — the fast receipt, before the party thinks.
                if let Some(ack) = ack {
                    let _ = self.transport.send(&self_address, &reply_to, ack).await;
                }
                // The party's state change completes before its reply is sent
                // (decision 17); kameo's mailbox serializes turns per actor.
                let reply = match &mut self.party {
                    Some(party) => party.on_turn_boxed(&reply_to, turn).await,
                    None => None,
                };
                if let Some(reply_turn) = reply {
                    let mut state = self.turns.get(&reply_to).copied().unwrap_or_default();
                    if state.on_outgoing(&reply_turn).is_ok() {
                        self.turns.insert(reply_to.clone(), state);
                        let _ = self
                            .transport
                            .send(&self_address, &reply_to, Vocab::Turn(reply_turn))
                            .await;
                    }
                }
            }
        }
    }
}

impl<T, R> Host<T, R>
where
    T: Transport + Clone + Send + Sync + 'static,
    R: Registry + Send + 'static,
    R::Inbox: Send + 'static,
{
    /// Execute a harness command via the runtime. Mirrors
    /// `LocalRuntime::execute`; reply kinds arriving here are ignored.
    fn execute(&self, command: Harness, controller: &Address) -> Option<Vocab> {
        let reply = match command {
            Harness::Ping => Harness::Pong,
            Harness::Spawn { name } => {
                // The spawned actor admits the controlling endpoint only,
                // and gets an EchoParty behind it — the first real Party.
                let mut allowlist = Allowlist::empty();
                allowlist.admit(controller.endpoint.clone());
                match self.runtime.install_party(
                    name.clone(),
                    allowlist,
                    Box::new(EchoParty::new()),
                ) {
                    Ok(()) => Harness::Spawned { name },
                    Err(e) => Harness::Failed {
                        reason: e.to_string(),
                    },
                }
            }
            Harness::Stop { name } => match self.runtime.stop(&name) {
                Ok(()) => Harness::Stopped { name },
                Err(_) => Harness::Failed {
                    reason: "no such actor".into(),
                },
            },
            Harness::Spawned { .. }
            | Harness::Stopped { .. }
            | Harness::Pong
            | Harness::Failed { .. } => return None,
        };
        Some(Vocab::Harness(reply))
    }
}
