//! The harness conversation, re-run against real Kameo actors.
//!
//! Same shape as the root crate's `harness_conversation` test, with one
//! crucial difference: no manual `step()`. Actors run autonomously — the
//! runtime's pump tasks and Kameo mailboxes do the driving, and the
//! controller simply awaits replies. This is the engine-for-engine proof.

use std::time::Duration;

use kamiroh_adapter_kameo::KameoRuntime;
use kamiroh_adapter_memory::{MemoryNet, MemoryTransportError};
use kamiroh_app::conversation::Conversation;
use kamiroh_app::inbound::{Inbound, process};
use kamiroh_app::runtime::ActorKind;
use kamiroh_domain::actor::{ActorName, Address};
use kamiroh_domain::allowlist::Allowlist;
use kamiroh_domain::endpoint::EndpointId;
use kamiroh_domain::hex::Hex;
use kamiroh_domain::vocabulary::{Harness, Message, Request, RequestId};
use kamiroh_ports::{Inbox, Transport};
use tokio::time::timeout;

fn endpoint(s: &str) -> EndpointId {
    EndpointId::new(Hex::new(s).unwrap())
}

fn name(s: &str) -> ActorName {
    ActorName::new(s).unwrap()
}

fn address(e: &str, n: &str) -> Address {
    Address::new(endpoint(e), name(n))
}

/// Await the controller's next admitted inbound, with a deadline so a hung
/// pump fails the test instead of wedging it.
async fn next_inbound(
    inbox: &mut kamiroh_adapter_memory::MemoryInbox,
    allowlist: &Allowlist,
) -> Inbound {
    let delivery = timeout(Duration::from_secs(5), inbox.next())
        .await
        .expect("timed out waiting for a delivery")
        .expect("inbox closed");
    process(allowlist, delivery)
}

#[tokio::test]
async fn kameo_actors_run_the_harness_conversation() {
    let net = MemoryNet::new();
    let mut transport = net.transport();

    // Controller actor on endpoint "aa" — driving side, hand-rolled.
    let controller = address("aa", "controller");
    let mut controller_inbox = net.register(controller.clone()).unwrap();
    let mut controller_list = Allowlist::empty();
    controller_list.admit(endpoint("bb"));

    // Endpoint "bb" runs the Kameo runtime; harness actor admits "aa".
    let runtime = KameoRuntime::new(endpoint("bb"), net.transport(), net.clone());
    let mut harness_list = Allowlist::empty();
    harness_list.admit(controller.endpoint.clone());
    runtime
        .install(name("harness"), harness_list, ActorKind::Harness)
        .unwrap();
    let harness = address("bb", "harness");

    let mut conv = Conversation::new(harness.clone());

    // -- Exchange 1: ping → pong (autonomous: no step calls) -------------
    let ping = Message::Harness(Harness::Ping);
    conv.begin(&ping).unwrap();
    transport.send(&controller, &harness, ping).await.unwrap();
    let Inbound::Harness { harness: reply, .. } =
        next_inbound(&mut controller_inbox, &controller_list).await
    else {
        panic!("expected a harness reply");
    };
    assert_eq!(reply, Harness::Pong);
    conv.conclude(&Message::Harness(reply)).unwrap();

    // -- Exchange 2: spawn echo-1 → spawned -------------------------------
    let spawn = Message::Harness(Harness::Spawn {
        name: name("echo-1"),
    });
    conv.begin(&spawn).unwrap();
    transport.send(&controller, &harness, spawn).await.unwrap();
    let Inbound::Harness { harness: reply, .. } =
        next_inbound(&mut controller_inbox, &controller_list).await
    else {
        panic!("expected a harness reply");
    };
    assert_eq!(
        reply,
        Harness::Spawned {
            name: name("echo-1")
        }
    );
    conv.conclude(&Message::Harness(reply)).unwrap();

    // -- Second conversation: request-ack with the spawned actor ----------
    let echo = address("bb", "echo-1");
    let mut echo_conv = Conversation::new(echo.clone());
    let request = Message::Request(Request {
        id: RequestId([4; 16]),
        body: b"are you alive?".to_vec(),
    });
    echo_conv.begin(&request).unwrap();
    transport.send(&controller, &echo, request).await.unwrap();
    let Inbound::AckReceived(ack) = next_inbound(&mut controller_inbox, &controller_list).await
    else {
        panic!("expected an ack");
    };
    assert_eq!(ack.id, RequestId([4; 16]));
    echo_conv.conclude(&Message::Ack(ack)).unwrap();

    // -- Exchange 3: stop echo-1 → stopped --------------------------------
    let stop = Message::Harness(Harness::Stop {
        name: name("echo-1"),
    });
    conv.begin(&stop).unwrap();
    transport.send(&controller, &harness, stop).await.unwrap();
    let Inbound::Harness { harness: reply, .. } =
        next_inbound(&mut controller_inbox, &controller_list).await
    else {
        panic!("expected a harness reply");
    };
    assert_eq!(
        reply,
        Harness::Stopped {
            name: name("echo-1")
        }
    );
    conv.conclude(&Message::Harness(reply)).unwrap();

    // The stopped actor unbinds asynchronously (pump abort drops its inbox);
    // poll briefly rather than racing it.
    let mut unbound = false;
    for _ in 0..50 {
        let err = transport
            .send(
                &controller,
                &echo,
                Message::Request(Request {
                    id: RequestId([5; 16]),
                    body: vec![],
                }),
            )
            .await;
        if err == Err(MemoryTransportError::UnknownAddress) {
            unbound = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(unbound, "stopped actor's address was never unbound");
}

#[tokio::test]
async fn unadmitted_commands_are_dropped_by_kameo_hosts() {
    let net = MemoryNet::new();
    let mut transport = net.transport();

    let mallory = address("cc", "mallory");
    let _mallory_inbox = net.register(mallory.clone()).unwrap();

    let runtime = KameoRuntime::new(endpoint("bb"), net.transport(), net.clone());
    let mut harness_list = Allowlist::empty();
    harness_list.admit(endpoint("aa")); // not Mallory's endpoint
    runtime
        .install(name("harness"), harness_list, ActorKind::Harness)
        .unwrap();

    transport
        .send(
            &mallory,
            &address("bb", "harness"),
            Message::Harness(Harness::Spawn { name: name("evil") }),
        )
        .await
        .unwrap();

    // Give the pump and host ample time to (not) act, then verify: no actor
    // spawned at the requested address.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let err = transport
        .send(
            &mallory,
            &address("bb", "evil"),
            Message::Harness(Harness::Ping),
        )
        .await;
    assert_eq!(err, Err(MemoryTransportError::UnknownAddress));
}
