//! End-to-end proof of the request-ack protocol over the in-process
//! transport: two actors on two (simulated) endpoints, every delivery passing
//! through the application layer's inbound processing.

use kamiroh::adapter_memory::MemoryNet;
use kamiroh::adapter_memory::testing::block_on;
use kamiroh::app::inbound::{Inbound, process};
use kamiroh::domain::actor::{ActorName, Address};
use kamiroh::domain::allowlist::Allowlist;
use kamiroh::domain::endpoint::EndpointId;
use kamiroh::domain::hex::Hex;
use kamiroh::domain::vocabulary::{Message, Request, RequestId};
use kamiroh::ports::{Inbox, Transport};

fn endpoint(s: &str) -> EndpointId {
    EndpointId::new(Hex::new(s).unwrap())
}

fn address(e: &str, name: &str) -> Address {
    Address::new(endpoint(e), ActorName::new(name).unwrap())
}

#[test]
fn request_ack_end_to_end() {
    block_on(async {
        let net = MemoryNet::new();
        let alice = address("aa", "alice");
        let bob = address("bb", "bob");
        let mut alice_inbox = net.register(alice.clone()).unwrap();
        let mut bob_inbox = net.register(bob.clone()).unwrap();
        let mut transport = net.transport();

        // Mutual admission, endpoint-only.
        let mut alice_list = Allowlist::empty();
        alice_list.admit(bob.endpoint.clone());
        let mut bob_list = Allowlist::empty();
        bob_list.admit(alice.endpoint.clone());

        // Alice sends a request to Bob.
        let request = Request {
            id: RequestId([7; 16]),
            body: b"hello, bob".to_vec(),
        };
        transport
            .send(&alice, &bob, Message::Request(request.clone()))
            .await
            .unwrap();

        // Bob's side processes the delivery; the app layer hands him the
        // request and a ready-made ack to return.
        let delivery = bob_inbox.next().await.unwrap();
        let Inbound::Request {
            request: received,
            for_actor,
            reply_to,
            ack,
        } = process(&bob_list, delivery)
        else {
            panic!("expected an admitted request");
        };
        assert_eq!(received, request);
        assert_eq!(for_actor, bob);
        transport.send(&bob, &reply_to, ack).await.unwrap();

        // Alice's side processes the ack: her request reached Bob's actor.
        let delivery = alice_inbox.next().await.unwrap();
        let Inbound::AckReceived(ack) = process(&alice_list, delivery) else {
            panic!("expected an admitted ack");
        };
        assert_eq!(ack.id, request.id);
    });
}

#[test]
fn unadmitted_endpoint_is_denied_end_to_end() {
    block_on(async {
        let net = MemoryNet::new();
        let bob = address("bb", "bob");
        let mallory = address("cc", "mallory");
        let mut bob_inbox = net.register(bob.clone()).unwrap();
        let mut transport = net.transport();

        // Bob admits only endpoint "aa"; Mallory is on "cc".
        let mut bob_list = Allowlist::empty();
        bob_list.admit(endpoint("aa"));

        transport
            .send(
                &mallory,
                &bob,
                Message::Request(Request {
                    id: RequestId([9; 16]),
                    body: b"let me in".to_vec(),
                }),
            )
            .await
            .unwrap();

        // The transport delivered it (the memory adapter lets tests claim any
        // origin), but the app layer refuses it at the choke point.
        let delivery = bob_inbox.next().await.unwrap();
        assert_eq!(process(&bob_list, delivery), Inbound::Denied);
    });
}
