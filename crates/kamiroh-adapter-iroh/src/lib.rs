//! Iroh transport adapter.
//!
//! Implements the [`Transport`] and [`Registry`] ports on real Iroh
//! connections: conversations travel as QUIC streams between endpoints
//! identified by public keys. This is the adapter kamiroh is named for.
//!
//! Design (`ARCHITECTURE.md`, decision 19):
//! - **Origin is proven, names are claimed.** The receiving side constructs
//!   `Delivery::from.endpoint` from the connection's authenticated remote
//!   key — never from frame content. Only the *name* halves ride in the
//!   frame. Forging an origin endpoint is therefore impossible; forging a
//!   name means only what the trust model already says it means.
//! - **Static peer book.** Discovery is deferred; peers are introduced
//!   explicitly via [`IrohNet::add_peer`] (matching "static configuration
//!   for the spike").
//! - **One frame per uni-stream** over a cached per-peer connection, one
//!   reconnect retry on stale connections. ALPN `kamiroh/0`. Wire format:
//!   length-implicit postcard (the stream is the frame boundary).
//! - Allowlist enforcement stays where it lives: the app layer's per-delivery
//!   admission. The adapter delivers to bound names and does nothing more.
//!
//! ## Version assumptions — read me on the first local build pass
//!
//! Written against `iroh = "0.35"`-era APIs **without compiling** (the cloud
//! sandbox cannot reach crates.io). Points most likely to need adjustment,
//! in probable order of drift (calibration: on the kameo round, 4 of 5
//! guesses held; the entry-point call was the one that moved):
//!
//! 1. Endpoint construction: `Endpoint::builder().secret_key(..)
//!    .alpns(vec![ALPN.to_vec()]).relay_mode(RelayMode::Disabled)
//!    .bind().await` — builder method names and whether `bind` takes args.
//! 2. Getting our dialable address: `endpoint.node_addr()` may be a watcher
//!    (`.initialized().await`) rather than a direct getter; the type may be
//!    `NodeAddr` or `EndpointAddr` depending on version.
//! 3. `endpoint.connect(addr, ALPN).await` argument types (NodeAddr vs
//!    NodeId-with-peer-book-inside-iroh).
//! 4. Remote identity: `connection.remote_node_id()` (name/fallibility).
//! 5. Stream API: `open_uni` / `accept_uni`, `write_all` + `finish` (+
//!    whether `finish` is sync), `read_to_end(limit)`.
//! 6. Accept loop: `endpoint.accept().await` yields an `Incoming` that is
//!    itself awaited to a `Connection` (possibly via `.accept()?`).
//! 7. Key types: `SecretKey::from_bytes(&[u8; 32])`, `NodeId::from_bytes`,
//!    hex `Display`/`FromStr` for NodeId.
//!
//! The routing, framing, peer book, and trust structure are iroh-agnostic
//! and should not need touching.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

// iroh 1.0 renamed NodeId -> EndpointId and NodeAddr -> EndpointAddr. Alias
// them back to the adapter's names, which stay distinct from the domain's own
// `EndpointId` and keep the routing/framing code below unchanged.
use iroh::endpoint::presets;
use iroh::{
    Endpoint, EndpointAddr as NodeAddr, EndpointId as NodeId, RelayMode, SecretKey, Watcher,
};
use tokio::sync::mpsc;

use kamiroh_domain::actor::{ActorName, Address};
use kamiroh_domain::endpoint::EndpointId;
use kamiroh_domain::hex::Hex;
use kamiroh_domain::secret::Secret;
use kamiroh_domain::vocabulary::Message;
use kamiroh_ports::{Delivery, Inbox, Registry, Transport};

/// The kamiroh ALPN: one protocol version on the wire.
pub const ALPN: &[u8] = b"kamiroh/0";

/// Cap on a single frame; a spike guard, not a protocol constant.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// What crosses the wire. The origin *endpoint* deliberately does not:
/// the receiver takes it from the connection's authenticated remote key.
#[derive(serde::Serialize, serde::Deserialize)]
struct Frame {
    from_name: ActorName,
    to_name: ActorName,
    message: Message,
}

#[derive(Default)]
struct Router {
    /// Bound local actors: name → sender into that actor's inbox.
    bound: HashMap<ActorName, mpsc::UnboundedSender<Delivery>>,
}

struct Shared {
    endpoint: Endpoint,
    endpoint_id: EndpointId,
    router: Mutex<Router>,
    /// Static peer book: domain endpoint id → dialable iroh address.
    peers: Mutex<HashMap<EndpointId, NodeAddr>>,
    /// Cached connections per peer.
    connections: tokio::sync::Mutex<HashMap<EndpointId, iroh::endpoint::Connection>>,
}

/// One kamiroh endpoint on the Iroh network: owns the iroh `Endpoint`, the
/// local actor router, and the accept loop. Clone handles freely.
#[derive(Clone)]
pub struct IrohNet {
    shared: Arc<Shared>,
    /// Held by the founding handle set; aborting ends the accept loop.
    accept_loop: Arc<AbortOnDrop>,
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// How an endpoint meets the network (`ARCHITECTURE.md`, decision 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetProfile {
    /// Relay-less and lookup-less: static peer book only
    /// (`presets::Minimal` + `RelayMode::Disabled`). The default — tests,
    /// closed deployments, anything hermetic.
    #[default]
    Hermetic,
    /// n0's public infrastructure (`presets::N0`): relay fleet for
    /// rendezvous/fallback, address publishing + lookup so peers dial by
    /// endpoint id alone. NATs — however many layers — are Iroh's problem,
    /// not the operator's.
    N0,
}

impl IrohNet {
    /// Bind a [`NetProfile::Hermetic`] endpoint from domain [`Secret`] key
    /// material (32 bytes) and start the accept loop.
    pub async fn bind(secret: &Secret) -> Result<Self, IrohNetError> {
        Self::bind_inner(secret, NetProfile::Hermetic, None).await
    }

    /// Like [`IrohNet::bind`], but listening on a fixed UDP port — for
    /// relay-less endpoints that must be dialable at a pre-arranged address
    /// (a port-forwarded router, a container with a published port). Under
    /// [`NetProfile::N0`] a fixed port is unnecessary: dial by id instead.
    pub async fn bind_on(secret: &Secret, port: Option<u16>) -> Result<Self, IrohNetError> {
        Self::bind_inner(secret, NetProfile::Hermetic, port).await
    }

    /// Bind with an explicit [`NetProfile`].
    pub async fn bind_with(secret: &Secret, profile: NetProfile) -> Result<Self, IrohNetError> {
        Self::bind_inner(secret, profile, None).await
    }

    async fn bind_inner(
        secret: &Secret,
        profile: NetProfile,
        port: Option<u16>,
    ) -> Result<Self, IrohNetError> {
        let bytes: [u8; 32] = secret
            .expose()
            .try_into()
            .map_err(|_| IrohNetError::BadSecret)?;
        let secret_key = SecretKey::from_bytes(&bytes);
        let mut builder = match profile {
            NetProfile::Hermetic => {
                Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
            }
            NetProfile::N0 => Endpoint::builder(presets::N0),
        };
        if let Some(port) = port {
            builder = builder
                .bind_addr(format!("0.0.0.0:{port}"))
                .map_err(|e| IrohNetError::Bind(e.to_string()))?;
        }
        let endpoint = builder
            .secret_key(secret_key)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| IrohNetError::Bind(e.to_string()))?;

        let endpoint_id = node_id_to_endpoint_id(endpoint.id());
        let shared = Arc::new(Shared {
            endpoint,
            endpoint_id,
            router: Mutex::new(Router::default()),
            peers: Mutex::new(HashMap::new()),
            connections: tokio::sync::Mutex::new(HashMap::new()),
        });

        let accept_shared = Arc::clone(&shared);
        let accept_loop = tokio::spawn(async move {
            accept_loop(accept_shared).await;
        });

        Ok(Self {
            shared,
            accept_loop: Arc::new(AbortOnDrop(accept_loop)),
        })
    }

    /// This endpoint's domain identity.
    pub fn endpoint_id(&self) -> &EndpointId {
        &self.shared.endpoint_id
    }

    /// This endpoint's dialable address, for handing to peers' `add_peer`.
    pub async fn addr(&self) -> Result<NodeAddr, IrohNetError> {
        // iroh 1.0: `addr()` is a plain getter over a watcher whose direct
        // addresses populate shortly after bind. Wait for the first non-empty
        // set so the peer book never caches an undialable address.
        let mut watcher = self.shared.endpoint.watch_addr();
        loop {
            let addr = watcher.get();
            if !addr.addrs.is_empty() {
                return Ok(addr);
            }
            watcher
                .updated()
                .await
                .map_err(|e| IrohNetError::Addr(format!("{e:?}")))?;
        }
    }

    /// Introduce a peer by endpoint id alone — usable under
    /// [`NetProfile::N0`], where address lookup resolves the id to a path
    /// (relay and, when hole-punching succeeds, direct).
    pub fn add_peer_by_id(&self, id: &EndpointId) -> Result<(), IrohNetError> {
        let node_id = endpoint_id_to_node_id(id)?;
        self.add_peer(NodeAddr::new(node_id));
        Ok(())
    }

    /// A one-line description of the live network paths to `peer`, if a
    /// connection exists — diagnostic sugar for checks ("did hole-punching
    /// win, or are we relaying?").
    pub async fn paths_to(&self, peer: &EndpointId) -> Option<String> {
        let connections = self.shared.connections.lock().await;
        connections.get(peer).map(|c| format!("{:?}", c.paths()))
    }

    /// Introduce a peer: static addressing, per the deferred-discovery
    /// decision. Returns the peer's domain endpoint id.
    pub fn add_peer(&self, addr: NodeAddr) -> EndpointId {
        let id = node_id_to_endpoint_id(addr.id);
        self.shared
            .peers
            .lock()
            .expect("peers poisoned")
            .insert(id.clone(), addr);
        id
    }

    /// A [`Transport`] handle onto the network.
    pub fn transport(&self) -> IrohTransport {
        IrohTransport {
            shared: Arc::clone(&self.shared),
            _accept_loop: Arc::clone(&self.accept_loop),
        }
    }
}

impl Registry for IrohNet {
    type Inbox = IrohInbox;
    type Error = IrohNetError;

    fn bind(&mut self, address: &Address) -> Result<Self::Inbox, Self::Error> {
        if address.endpoint != self.shared.endpoint_id {
            return Err(IrohNetError::WrongEndpoint);
        }
        let mut router = self.shared.router.lock().expect("router poisoned");
        if let Some(existing) = router.bound.get(&address.name)
            && !existing.is_closed()
        {
            return Err(IrohNetError::NameInUse);
        }
        let (tx, rx) = mpsc::unbounded_channel();
        router.bound.insert(address.name.clone(), tx);
        Ok(IrohInbox { rx })
    }
}

/// [`Inbox`] over the accept loop's routing. Dropping it closes the channel;
/// the router prunes the binding lazily (a closed sender may be rebound).
pub struct IrohInbox {
    rx: mpsc::UnboundedReceiver<Delivery>,
}

impl Inbox for IrohInbox {
    async fn next(&mut self) -> Option<Delivery> {
        self.rx.recv().await
    }
}

/// [`Transport`] implementation: one postcard frame per uni-stream, cached
/// connections, one retry on stale connections.
#[derive(Clone)]
pub struct IrohTransport {
    shared: Arc<Shared>,
    _accept_loop: Arc<AbortOnDrop>,
}

impl Transport for IrohTransport {
    type Error = IrohTransportError;

    async fn send(
        &mut self,
        from: &Address,
        to: &Address,
        message: Message,
    ) -> Result<(), Self::Error> {
        if from.endpoint != self.shared.endpoint_id {
            return Err(IrohTransportError::NotOurEndpoint);
        }
        let frame = Frame {
            from_name: from.name.clone(),
            to_name: to.name.clone(),
            message,
        };
        let bytes =
            postcard::to_stdvec(&frame).map_err(|e| IrohTransportError::Encode(e.to_string()))?;

        // One retry: a cached connection may have gone stale.
        let mut last_err = None;
        for attempt in 0..2 {
            let connection = self.connection_to(&to.endpoint, attempt > 0).await?;
            match send_frame(&connection, &bytes).await {
                Ok(()) => return Ok(()),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.expect("two attempts made"))
    }
}

impl IrohTransport {
    async fn connection_to(
        &self,
        peer: &EndpointId,
        force_fresh: bool,
    ) -> Result<iroh::endpoint::Connection, IrohTransportError> {
        let mut connections = self.shared.connections.lock().await;
        if !force_fresh && let Some(existing) = connections.get(peer) {
            return Ok(existing.clone());
        }
        let addr = self
            .shared
            .peers
            .lock()
            .expect("peers poisoned")
            .get(peer)
            .cloned()
            .ok_or(IrohTransportError::UnknownPeer)?;
        let connection = self
            .shared
            .endpoint
            .connect(addr, ALPN)
            .await
            .map_err(|e| IrohTransportError::Connect(e.to_string()))?;
        connections.insert(peer.clone(), connection.clone());
        // Streams the peer opens on THIS connection (e.g. replies) arrive
        // here, not at the accept loop — every connection gets a reader,
        // whichever side dialed it.
        spawn_reader(Arc::clone(&self.shared), connection.clone());
        Ok(connection)
    }
}

async fn send_frame(
    connection: &iroh::endpoint::Connection,
    bytes: &[u8],
) -> Result<(), IrohTransportError> {
    let mut stream = connection
        .open_uni()
        .await
        .map_err(|e| IrohTransportError::Stream(e.to_string()))?;
    stream
        .write_all(bytes)
        .await
        .map_err(|e| IrohTransportError::Stream(e.to_string()))?;
    // Assumption point 5: finish may be sync or async across versions.
    stream
        .finish()
        .map_err(|e| IrohTransportError::Stream(e.to_string()))?;
    // Give the peer a chance to read before the connection is dropped by a
    // short-lived sender: await stream close acknowledgment.
    let _ = stream.stopped().await;
    Ok(())
}

async fn accept_loop(shared: Arc<Shared>) {
    // Assumption point 6: accept() → Incoming → Connection.
    while let Some(incoming) = shared.endpoint.accept().await {
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else {
                return;
            };
            // An inbound connection teaches us the peer: cache it so replies
            // flow back over the very connection the request arrived on. A
            // receiving endpoint therefore needs no peer-book entry for its
            // callers — admission still gates every delivery at the app
            // layer. (Found by the Incus-check rehearsal: without this, a
            // server could hear but never answer an unknown-address caller.)
            let origin = node_id_to_endpoint_id(connection.remote_id());
            shared
                .connections
                .lock()
                .await
                .insert(origin, connection.clone());
            spawn_reader(shared, connection);
        });
    }
}

/// Read frames off one connection for its lifetime, routing deliveries to
/// bound actors. Spawned for every connection — accepted *or* dialed —
/// because QUIC is bidirectional: the peer may open streams on either.
fn spawn_reader(shared: Arc<Shared>, connection: iroh::endpoint::Connection) {
    tokio::spawn(async move {
        // The proven origin: the connection's authenticated remote key.
        // iroh 1.0: `remote_id()` on an established Connection is infallible.
        let origin = node_id_to_endpoint_id(connection.remote_id());
        loop {
            let Ok(mut stream) = connection.accept_uni().await else {
                return; // connection closed
            };
            let Ok(bytes) = stream.read_to_end(MAX_FRAME_BYTES).await else {
                continue;
            };
            let Ok(frame) = postcard::from_bytes::<Frame>(&bytes) else {
                continue; // malformed frame: drop
            };
            let delivery = Delivery {
                from: Address::new(origin.clone(), frame.from_name),
                to: Address::new(shared.endpoint_id.clone(), frame.to_name),
                message: frame.message,
            };
            let router = shared.router.lock().expect("router poisoned");
            if let Some(tx) = router.bound.get(&delivery.to.name) {
                // Unknown or closed bindings drop silently: an unbound
                // name discloses nothing.
                let _ = tx.send(delivery);
            }
        }
    });
}

fn node_id_to_endpoint_id(id: NodeId) -> EndpointId {
    EndpointId::new(Hex::new(format!("{id}")).expect("node id displays as hex"))
}

/// Convert a domain endpoint id back to an iroh node id (peer-book helper).
pub fn endpoint_id_to_node_id(id: &EndpointId) -> Result<NodeId, IrohNetError> {
    NodeId::from_str(id.as_hex().as_str()).map_err(|_| IrohNetError::BadEndpointId)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrohNetError {
    /// Secrets must be exactly 32 bytes of key material.
    BadSecret,
    /// An endpoint id that is not a valid public key.
    BadEndpointId,
    /// Binding the iroh endpoint failed.
    Bind(String),
    /// Obtaining our dialable address failed.
    Addr(String),
    /// The address belongs to a different endpoint than this net.
    WrongEndpoint,
    /// An actor with this name is already bound.
    NameInUse,
}

impl fmt::Display for IrohNetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrohNetError::BadSecret => f.write_str("secret must be 32 bytes of key material"),
            IrohNetError::BadEndpointId => f.write_str("endpoint id is not a valid public key"),
            IrohNetError::Bind(e) => write!(f, "binding endpoint failed: {e}"),
            IrohNetError::Addr(e) => write!(f, "obtaining endpoint address failed: {e}"),
            IrohNetError::WrongEndpoint => {
                f.write_str("address belongs to a different endpoint than this net")
            }
            IrohNetError::NameInUse => f.write_str("an actor with this name is already bound"),
        }
    }
}

impl std::error::Error for IrohNetError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrohTransportError {
    /// `from` must be an address on this endpoint.
    NotOurEndpoint,
    /// The target endpoint is not in the peer book.
    UnknownPeer,
    Encode(String),
    Connect(String),
    Stream(String),
}

impl fmt::Display for IrohTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrohTransportError::NotOurEndpoint => {
                f.write_str("from-address is not on this endpoint")
            }
            IrohTransportError::UnknownPeer => {
                f.write_str("target endpoint is not in the peer book")
            }
            IrohTransportError::Encode(e) => write!(f, "frame encoding failed: {e}"),
            IrohTransportError::Connect(e) => write!(f, "connecting failed: {e}"),
            IrohTransportError::Stream(e) => write!(f, "stream failed: {e}"),
        }
    }
}

impl std::error::Error for IrohTransportError {}
