//! A deliberately small, single-endpoint actor runtime.
//!
//! This is the toy that `kamiroh-adapter-kameo` will replace: it owns the
//! endpoint's local actors, binds them through the [`Registry`] port, routes
//! each delivery through [`inbound::process`](crate::inbound::process), and
//! executes harness commands. Its value is fixing the *shape* — what owning
//! actors and routing deliveries means — against the memory transport, so the
//! Kameo adapter later swaps the engine, not the design.

use std::collections::HashMap;
use std::fmt;

use kamiroh_domain::actor::{ActorName, Address};
use kamiroh_domain::allowlist::Allowlist;
use kamiroh_domain::endpoint::EndpointId;
use kamiroh_domain::protocol::TurnState;
use kamiroh_domain::vocabulary::{Harness, Message};
use kamiroh_ports::{DynParty, Inbox, Registry, Transport};

use crate::inbound::{Inbound, process};
use crate::parties::EchoParty;

/// What kind of party sits behind a local actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    /// Interprets harness commands. Privileged (`ARCHITECTURE.md`, decision 6).
    Harness,
    /// An ordinary actor: receives requests, acks them. In v0 no party is
    /// wired behind it yet — the ack (delivery, not answer) is the point.
    Plain,
}

struct LocalActor<I> {
    inbox: I,
    allowlist: Allowlist,
    kind: ActorKind,
    /// The party behind this actor, if one is wired (decision 16).
    party: Option<Box<dyn DynParty>>,
    /// Per-conversation turn state, keyed by peer (decision 17).
    turns: HashMap<Address, TurnState>,
}

/// The toy runtime for one endpoint.
pub struct LocalRuntime<T: Transport, R: Registry> {
    endpoint: EndpointId,
    transport: T,
    registry: R,
    actors: HashMap<ActorName, LocalActor<R::Inbox>>,
}

impl<T: Transport, R: Registry> LocalRuntime<T, R> {
    pub fn new(endpoint: EndpointId, transport: T, registry: R) -> Self {
        Self {
            endpoint,
            transport,
            registry,
            actors: HashMap::new(),
        }
    }

    pub fn endpoint(&self) -> &EndpointId {
        &self.endpoint
    }

    /// Install an actor: bind its address via the [`Registry`] port and hold
    /// its inbox. Dropping the actor (see harness `Stop`) unbinds it.
    pub fn install(
        &mut self,
        name: ActorName,
        allowlist: Allowlist,
        kind: ActorKind,
    ) -> Result<(), RuntimeError> {
        self.install_inner(name, allowlist, kind, None)
    }

    /// Install an actor with the party behind it (decision 16): incoming
    /// turns are handed to `party`, its returned turns sent for it.
    pub fn install_party(
        &mut self,
        name: ActorName,
        allowlist: Allowlist,
        party: Box<dyn DynParty>,
    ) -> Result<(), RuntimeError> {
        self.install_inner(name, allowlist, ActorKind::Plain, Some(party))
    }

    fn install_inner(
        &mut self,
        name: ActorName,
        allowlist: Allowlist,
        kind: ActorKind,
        party: Option<Box<dyn DynParty>>,
    ) -> Result<(), RuntimeError> {
        if self.actors.contains_key(&name) {
            return Err(RuntimeError::NameInUse);
        }
        let address = Address::new(self.endpoint.clone(), name.clone());
        let inbox = self
            .registry
            .bind(&address)
            .map_err(|e| RuntimeError::Bind(e.to_string()))?;
        self.actors.insert(
            name,
            LocalActor {
                inbox,
                allowlist,
                kind,
                party,
                turns: HashMap::new(),
            },
        );
        Ok(())
    }

    /// Take the next delivery for `name`'s actor and act on it: enforce
    /// admission, ack admitted requests, execute harness commands. One
    /// delivery per call, so tests stay deterministic.
    pub async fn step(&mut self, name: &ActorName) -> Result<(), RuntimeError> {
        let actor = self
            .actors
            .get_mut(name)
            .ok_or(RuntimeError::UnknownActor)?;
        let allowlist = actor.allowlist.clone();
        let kind = actor.kind;
        let Some(delivery) = actor.inbox.next().await else {
            return Err(RuntimeError::InboxClosed);
        };
        let self_address = delivery.to.clone();
        match process(&allowlist, delivery) {
            Inbound::Denied => Ok(()),
            Inbound::Request { reply_to, ack, .. } => {
                // In v0 there is no party wired behind a Plain actor yet; the
                // request is received and the delivery acknowledged.
                self.send(&self_address, &reply_to, ack).await
            }
            Inbound::AckReceived(_) => Ok(()),
            Inbound::Harness { harness, reply_to } => {
                if kind != ActorKind::Harness {
                    let reply = Message::Harness(Harness::Failed {
                        reason: "not a harness actor".into(),
                    });
                    return self.send(&self_address, &reply_to, reply).await;
                }
                let reply = self.execute(harness, &reply_to);
                match reply {
                    Some(reply) => self.send(&self_address, &reply_to, reply).await,
                    None => Ok(()),
                }
            }
            Inbound::Turn {
                turn,
                for_actor: _,
                reply_to,
                ack,
            } => {
                // Validate against this conversation's alternation state;
                // illegal turns are dropped silently (protocol violation).
                let actor = self
                    .actors
                    .get_mut(name)
                    .ok_or(RuntimeError::UnknownActor)?;
                let mut state = actor.turns.get(&reply_to).copied().unwrap_or_default();
                if state.on_incoming(&turn).is_err() {
                    return Ok(());
                }
                actor.turns.insert(reply_to.clone(), state);
                // Ack on handover — the fast receipt, before the party thinks.
                if let Some(ack) = ack {
                    self.send(&self_address, &reply_to, ack).await?;
                }
                // Hand the turn to the party; its state change completes
                // before its reply exists, let alone is sent.
                let reply = {
                    let actor = self
                        .actors
                        .get_mut(name)
                        .ok_or(RuntimeError::UnknownActor)?;
                    match &mut actor.party {
                        Some(party) => party.on_turn_boxed(&reply_to, turn).await,
                        None => None,
                    }
                };
                if let Some(reply_turn) = reply {
                    let legal = {
                        let actor = self
                            .actors
                            .get_mut(name)
                            .ok_or(RuntimeError::UnknownActor)?;
                        let mut state = actor.turns.get(&reply_to).copied().unwrap_or_default();
                        let legal = state.on_outgoing(&reply_turn).is_ok();
                        if legal {
                            actor.turns.insert(reply_to.clone(), state);
                        }
                        legal
                    };
                    if legal {
                        return self
                            .send(&self_address, &reply_to, Message::Turn(reply_turn))
                            .await;
                    }
                }
                Ok(())
            }
        }
    }

    /// Execute a harness command, returning the reply to send. Reply kinds
    /// (`Spawned`, `Stopped`, `Pong`, `Failed`) arriving here are ignored.
    fn execute(&mut self, command: Harness, controller: &Address) -> Option<Message> {
        let reply = match command {
            Harness::Ping => Harness::Pong,
            Harness::Spawn { name } => {
                // The spawned actor admits the controlling endpoint only,
                // and gets an EchoParty behind it — the first real Party.
                let mut allowlist = Allowlist::empty();
                allowlist.admit(controller.endpoint.clone());
                match self.install_party(name.clone(), allowlist, Box::new(EchoParty::new())) {
                    Ok(()) => Harness::Spawned { name },
                    Err(e) => Harness::Failed {
                        reason: e.to_string(),
                    },
                }
            }
            Harness::Stop { name } => {
                // Dropping the actor drops its inbox, which unbinds the
                // address at the transport (Registry contract).
                match self.actors.remove(&name) {
                    Some(_) => Harness::Stopped { name },
                    None => Harness::Failed {
                        reason: "no such actor".into(),
                    },
                }
            }
            Harness::Spawned { .. }
            | Harness::Stopped { .. }
            | Harness::Pong
            | Harness::Failed { .. } => return None,
        };
        Some(Message::Harness(reply))
    }

    async fn send(
        &mut self,
        from: &Address,
        to: &Address,
        message: Message,
    ) -> Result<(), RuntimeError> {
        self.transport
            .send(from, to, message)
            .await
            .map_err(|e| RuntimeError::Transport(e.to_string()))
    }
}

/// Spike-pragmatic error type: adapter errors are carried as text rather than
/// generics, keeping the runtime's signature simple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    UnknownActor,
    NameInUse,
    InboxClosed,
    Bind(String),
    Transport(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::UnknownActor => f.write_str("no such actor in this runtime"),
            RuntimeError::NameInUse => f.write_str("an actor with this name is already installed"),
            RuntimeError::InboxClosed => f.write_str("the actor's inbox is closed"),
            RuntimeError::Bind(e) => write!(f, "binding failed: {e}"),
            RuntimeError::Transport(e) => write!(f, "transport failed: {e}"),
        }
    }
}

impl std::error::Error for RuntimeError {}
