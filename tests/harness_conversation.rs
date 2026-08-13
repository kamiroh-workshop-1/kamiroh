//! A multi-exchange conversation over the harness protocol.
//!
//! One controller actor converses with a remote endpoint's harness actor:
//! ping, spawn, stop — three exchanges, one after another, in one
//! conversation — with a request-ack exchange to the spawned actor (a second
//! conversation) in between. Exercises the sequential-exchange rule and the
//! Registry binding port along the way.

use kamiroh::adapter_memory::testing::block_on;
use kamiroh::adapter_memory::{MemoryNet, MemoryTransportError};
use kamiroh::app::conversation::{Conversation, ExchangeError};
use kamiroh::app::inbound::{Inbound, process};
use kamiroh::app::runtime::{ActorKind, LocalRuntime};
use kamiroh::domain::actor::{ActorName, Address};
use kamiroh::domain::allowlist::Allowlist;
use kamiroh::domain::endpoint::EndpointId;
use kamiroh::domain::hex::Hex;
use kamiroh::domain::vocabulary::{Harness, Message, Request, RequestId};
use kamiroh::ports::{Inbox, Transport};

fn endpoint(s: &str) -> EndpointId {
    EndpointId::new(Hex::new(s).unwrap())
}

fn name(s: &str) -> ActorName {
    ActorName::new(s).unwrap()
}

fn address(e: &str, n: &str) -> Address {
    Address::new(endpoint(e), name(n))
}

#[test]
fn a_conversation_of_sequential_exchanges() {
    block_on(async {
        let net = MemoryNet::new();
        let mut transport = net.transport();

        // Controller actor on endpoint "aa".
        let controller = address("aa", "controller");
        let mut controller_inbox = net.register(controller.clone()).unwrap();
        let mut controller_list = Allowlist::empty();
        controller_list.admit(endpoint("bb"));

        // Endpoint "bb" runs the toy runtime with a harness actor admitting
        // the controller's endpoint. Privileged grant, deliberately explicit.
        let mut runtime = LocalRuntime::new(endpoint("bb"), net.transport(), net.clone());
        let mut harness_list = Allowlist::empty();
        harness_list.admit(controller.endpoint.clone());
        runtime
            .install(name("harness"), harness_list, ActorKind::Harness)
            .unwrap();
        let harness = address("bb", "harness");

        // The conversation: controller ↔ harness@bb.
        let mut conv = Conversation::new(harness.clone());

        // -- Exchange 1: ping → pong ------------------------------------
        let ping = Message::Harness(Harness::Ping);
        conv.begin(&ping).unwrap();
        transport.send(&controller, &harness, ping).await.unwrap();

        // Sequential rule: a second exchange may not begin mid-flight.
        assert_eq!(
            conv.begin(&Message::Harness(Harness::Ping)),
            Err(ExchangeError::AlreadyInFlight)
        );

        runtime.step(&name("harness")).await.unwrap();
        let delivery = controller_inbox.next().await.unwrap();
        let Inbound::Harness { harness: reply, .. } = process(&controller_list, delivery) else {
            panic!("expected a harness reply");
        };
        assert_eq!(reply, Harness::Pong);
        conv.conclude(&Message::Harness(reply)).unwrap();

        // -- Exchange 2: spawn echo-1 → spawned --------------------------
        let spawn = Message::Harness(Harness::Spawn {
            name: name("echo-1"),
        });
        conv.begin(&spawn).unwrap();
        transport.send(&controller, &harness, spawn).await.unwrap();
        runtime.step(&name("harness")).await.unwrap();
        let delivery = controller_inbox.next().await.unwrap();
        let Inbound::Harness { harness: reply, .. } = process(&controller_list, delivery) else {
            panic!("expected a harness reply");
        };
        assert_eq!(
            reply,
            Harness::Spawned {
                name: name("echo-1")
            }
        );
        conv.conclude(&Message::Harness(reply)).unwrap();

        // -- A second conversation: request-ack with the spawned actor ---
        let echo = address("bb", "echo-1");
        let mut echo_conv = Conversation::new(echo.clone());
        let request = Message::Request(Request {
            id: RequestId([4; 16]),
            body: b"are you alive?".to_vec(),
        });
        echo_conv.begin(&request).unwrap();
        transport.send(&controller, &echo, request).await.unwrap();
        runtime.step(&name("echo-1")).await.unwrap();
        let delivery = controller_inbox.next().await.unwrap();
        let Inbound::AckReceived(ack) = process(&controller_list, delivery) else {
            panic!("expected an ack");
        };
        assert_eq!(ack.id, RequestId([4; 16]));
        echo_conv.conclude(&Message::Ack(ack)).unwrap();

        // -- Exchange 3 (first conversation): stop echo-1 → stopped ------
        let stop = Message::Harness(Harness::Stop {
            name: name("echo-1"),
        });
        conv.begin(&stop).unwrap();
        transport.send(&controller, &harness, stop).await.unwrap();
        runtime.step(&name("harness")).await.unwrap();
        let delivery = controller_inbox.next().await.unwrap();
        let Inbound::Harness { harness: reply, .. } = process(&controller_list, delivery) else {
            panic!("expected a harness reply");
        };
        assert_eq!(
            reply,
            Harness::Stopped {
                name: name("echo-1")
            }
        );
        conv.conclude(&Message::Harness(reply)).unwrap();

        // The stopped actor is unbound: the transport no longer knows it.
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
        assert_eq!(err, Err(MemoryTransportError::UnknownAddress));
    });
}

#[test]
fn harness_commands_to_unadmitted_controller_are_dropped() {
    block_on(async {
        let net = MemoryNet::new();
        let mut transport = net.transport();

        let mallory = address("cc", "mallory");
        let mut mallory_inbox = net.register(mallory.clone()).unwrap();

        // The harness admits only endpoint "aa"; Mallory is on "cc".
        let mut runtime = LocalRuntime::new(endpoint("bb"), net.transport(), net.clone());
        let mut harness_list = Allowlist::empty();
        harness_list.admit(endpoint("aa"));
        runtime
            .install(name("harness"), harness_list, ActorKind::Harness)
            .unwrap();
        let harness = address("bb", "harness");

        transport
            .send(
                &mallory,
                &harness,
                Message::Harness(Harness::Spawn { name: name("evil") }),
            )
            .await
            .unwrap();
        runtime.step(&name("harness")).await.unwrap();

        // Denied silently: no actor spawned, and no reply of any kind —
        // Mallory cannot even learn the harness exists.
        let err = transport
            .send(
                &mallory,
                &address("bb", "evil"),
                Message::Harness(Harness::Ping),
            )
            .await;
        assert_eq!(err, Err(MemoryTransportError::UnknownAddress));
        assert!(mallory_inbox.now_or_never_is_empty());
    });
}

/// Tiny helper: probe an inbox without blocking.
trait NowOrNever {
    fn now_or_never_is_empty(&mut self) -> bool;
}

impl NowOrNever for kamiroh::adapter_memory::MemoryInbox {
    fn now_or_never_is_empty(&mut self) -> bool {
        use std::future::Future;
        use std::pin::pin;
        use std::task::{Context, Poll, Waker};

        let mut cx = Context::from_waker(Waker::noop());
        let mut fut = pin!(self.next());
        matches!(fut.as_mut().poll(&mut cx), Poll::Pending)
    }
}
