//! The full stack over real sockets: Kameo actors hosted behind an Iroh
//! endpoint, a controller on a second endpoint, loopback QUIC between them.
//! Harness exchanges, a turn exchange with a spawned echo party, a
//! multi-round countdown exchange, and a denial proof — the spike's
//! headline: actors, addressable by name and endpoint, messaging across the
//! network to drive agents.

use std::time::Duration;

use kamiroh_adapter_iroh::{IrohInbox, IrohNet, IrohTransport};
use kamiroh_adapter_kameo::KameoRuntime;
use kamiroh_app::inbound::{Inbound, process};
use kamiroh_app::parties::CountdownParty;
use kamiroh_app::phone::Phone;
use kamiroh_app::runtime::ActorKind;
use kamiroh_domain::actor::{ActorName, Address};
use kamiroh_domain::allowlist::Allowlist;
use kamiroh_domain::protocol::TurnProgress;
use kamiroh_domain::secret::Secret;
use kamiroh_domain::vocabulary::{Harness, Message, Request, RequestId, Response, Turn};
use kamiroh_ports::{Inbox, Registry, Transport};
use tokio::time::timeout;

fn name(s: &str) -> ActorName {
    ActorName::new(s).unwrap()
}

fn request(n: u8) -> Request {
    Request {
        id: RequestId([n; 16]),
        body: vec![n],
    }
}

async fn next_inbound(inbox: &mut IrohInbox, allowlist: &Allowlist) -> Inbound {
    let delivery = timeout(Duration::from_secs(10), inbox.next())
        .await
        .expect("timed out waiting for a delivery")
        .expect("inbox closed");
    process(allowlist, delivery)
}

/// Await the next admitted *harness reply*, skipping acks.
async fn next_harness(inbox: &mut IrohInbox, allowlist: &Allowlist) -> Harness {
    loop {
        match next_inbound(inbox, allowlist).await {
            Inbound::AckReceived(_) => continue,
            Inbound::Harness { harness, .. } => return harness,
            other => panic!("expected a harness reply, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn actors_converse_across_real_iroh_endpoints() {
    // Two endpoints on loopback; relays and discovery disabled.
    let net_a = IrohNet::bind(&Secret::new(vec![1; 32])).await.unwrap();
    let net_b = IrohNet::bind(&Secret::new(vec![2; 32])).await.unwrap();

    // Static introduction, both directions (decision 19).
    let addr_a = net_a.addr().await.unwrap();
    let addr_b = net_b.addr().await.unwrap();
    let id_a = net_b.add_peer(addr_a);
    let id_b = net_a.add_peer(addr_b);
    assert_eq!(&id_a, net_a.endpoint_id());
    assert_eq!(&id_b, net_b.endpoint_id());

    // Endpoint B: Kameo runtime hosting a harness actor admitting A.
    let runtime = KameoRuntime::new(id_b.clone(), net_b.transport(), net_b.clone());
    let mut harness_list = Allowlist::empty();
    harness_list.admit(id_a.clone());
    runtime
        .install(name("harness"), harness_list, ActorKind::Harness)
        .unwrap();
    let harness = Address::new(id_b.clone(), name("harness"));

    // Endpoint A: a hand-rolled controller actor.
    let controller = Address::new(id_a.clone(), name("controller"));
    let mut net_a_registry = net_a.clone();
    let mut controller_inbox = net_a_registry.bind(&controller).unwrap();
    let mut controller_list = Allowlist::empty();
    controller_list.admit(id_b.clone());
    let mut transport: IrohTransport = net_a.transport();

    // -- Ping across the wire ------------------------------------------
    transport
        .send(&controller, &harness, Message::Harness(Harness::Ping))
        .await
        .unwrap();
    assert_eq!(
        next_harness(&mut controller_inbox, &controller_list).await,
        Harness::Pong
    );

    // -- Spawn an echo party remotely ----------------------------------
    transport
        .send(
            &controller,
            &harness,
            Message::Harness(Harness::Spawn { name: name("echo") }),
        )
        .await
        .unwrap();
    assert_eq!(
        next_harness(&mut controller_inbox, &controller_list).await,
        Harness::Spawned { name: name("echo") }
    );

    // -- A turn exchange with the spawned echo party --------------------
    let echo = Address::new(id_b.clone(), name("echo"));
    let mut phone = Phone::converse(controller.clone(), echo, net_a.transport());
    phone.open(request(7)).await.unwrap();
    let mut saw_ack = false;
    loop {
        match next_inbound(&mut controller_inbox, &controller_list).await {
            Inbound::AckReceived(ack) => {
                assert_eq!(ack.id, RequestId([7; 16]));
                saw_ack = true;
            }
            Inbound::Turn { turn, .. } => {
                assert_eq!(phone.on_incoming(&turn).unwrap(), TurnProgress::Concluded);
                let Turn::Close { response } = turn else {
                    panic!("echo should close immediately");
                };
                assert_eq!(response.body, vec![7]);
                break;
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
    }
    assert!(saw_ack, "delivery ack should precede the party's answer");

    // -- Multi-round turns across the wire ------------------------------
    let mut countdown_list = Allowlist::empty();
    countdown_list.admit(id_a.clone());
    runtime
        .install_party(
            name("counter"),
            countdown_list,
            Box::new(CountdownParty::new(2)),
        )
        .unwrap();
    let counter = Address::new(id_b.clone(), name("counter"));
    let mut phone = Phone::converse(controller.clone(), counter, net_a.transport());
    phone.open(request(1)).await.unwrap();

    let mut rounds = 0;
    let mut fresh: u8 = 10;
    loop {
        match next_inbound(&mut controller_inbox, &controller_list).await {
            Inbound::AckReceived(_) => continue,
            Inbound::Turn { turn, .. } => {
                let progress = phone.on_incoming(&turn).unwrap();
                match (progress, turn) {
                    (TurnProgress::Concluded, Turn::Close { .. }) => break,
                    (
                        TurnProgress::Continuing,
                        Turn::Continue {
                            request: theirs, ..
                        },
                    ) => {
                        rounds += 1;
                        let reply = Turn::Continue {
                            response: Response {
                                id: theirs.id,
                                body: theirs.body,
                            },
                            request: request(fresh),
                        };
                        fresh += 1;
                        phone.send_turn(reply).await.unwrap();
                    }
                    other => panic!("unexpected: {other:?}"),
                }
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
    }
    assert_eq!(rounds, 2, "countdown should pose two requests of its own");
}

#[tokio::test]
async fn one_sided_introduction_suffices_for_replies() {
    // Only the caller knows the server's address; the server has an EMPTY
    // peer book. Replies must ride back over the inbound connection
    // (learned-peer caching in the accept loop).
    let net_a = IrohNet::bind(&Secret::new(vec![6; 32])).await.unwrap();
    let net_b = IrohNet::bind(&Secret::new(vec![7; 32])).await.unwrap();

    let addr_b = net_b.addr().await.unwrap();
    let id_b = net_a.add_peer(addr_b);
    let id_a = net_a.endpoint_id().clone();
    // Deliberately NO net_b.add_peer(addr_a).

    let runtime = KameoRuntime::new(id_b.clone(), net_b.transport(), net_b.clone());
    let mut harness_list = Allowlist::empty();
    harness_list.admit(id_a.clone());
    runtime
        .install(name("harness"), harness_list, ActorKind::Harness)
        .unwrap();

    let controller = Address::new(id_a, name("controller"));
    let mut a_registry = net_a.clone();
    let mut inbox = a_registry.bind(&controller).unwrap();
    let mut controller_list = Allowlist::empty();
    controller_list.admit(id_b.clone());

    let mut transport = net_a.transport();
    transport
        .send(
            &controller,
            &Address::new(id_b, name("harness")),
            Message::Harness(Harness::Ping),
        )
        .await
        .unwrap();
    assert_eq!(
        next_harness(&mut inbox, &controller_list).await,
        Harness::Pong
    );
}

#[tokio::test]
async fn unadmitted_endpoints_are_denied_across_the_wire() {
    let net_a = IrohNet::bind(&Secret::new(vec![3; 32])).await.unwrap();
    let net_b = IrohNet::bind(&Secret::new(vec![4; 32])).await.unwrap();
    let net_c = IrohNet::bind(&Secret::new(vec![5; 32])).await.unwrap();

    let addr_b = net_b.addr().await.unwrap();
    let id_b = net_a.add_peer(addr_b.clone());
    net_c.add_peer(addr_b);
    let addr_a = net_a.addr().await.unwrap();
    let id_a = net_b.add_peer(addr_a);

    // B hosts a harness admitting only A. C is a stranger.
    let runtime = KameoRuntime::new(id_b.clone(), net_b.transport(), net_b.clone());
    let mut harness_list = Allowlist::empty();
    harness_list.admit(id_a.clone());
    runtime
        .install(name("harness"), harness_list, ActorKind::Harness)
        .unwrap();
    let harness = Address::new(id_b.clone(), name("harness"));

    // C attempts a spawn. The transport carries it (C can reach B's socket);
    // admission drops it silently — C gets nothing back, not even an error.
    let mallory = Address::new(net_c.endpoint_id().clone(), name("mallory"));
    let mut c_registry = net_c.clone();
    let mut mallory_inbox = c_registry.bind(&mallory).unwrap();
    let mut c_transport = net_c.transport();
    c_transport
        .send(
            &mallory,
            &harness,
            Message::Harness(Harness::Spawn { name: name("evil") }),
        )
        .await
        .unwrap();
    assert!(
        timeout(Duration::from_secs(2), mallory_inbox.next())
            .await
            .is_err(),
        "the unadmitted sender must hear only silence"
    );

    // Proof the spawn never happened: admitted A asks B's harness to stop
    // "evil" and is told there is no such actor.
    let controller = Address::new(id_a.clone(), name("controller"));
    let mut a_registry = net_a.clone();
    let mut controller_inbox = a_registry.bind(&controller).unwrap();
    let mut controller_list = Allowlist::empty();
    controller_list.admit(id_b.clone());
    let mut a_transport = net_a.transport();
    a_transport
        .send(
            &controller,
            &harness,
            Message::Harness(Harness::Stop { name: name("evil") }),
        )
        .await
        .unwrap();
    assert_eq!(
        next_harness(&mut controller_inbox, &controller_list).await,
        Harness::Failed {
            reason: "no such actor".into()
        }
    );
}
