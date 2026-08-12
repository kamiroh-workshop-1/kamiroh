//! A multi-round turn exchange: Phone on one side, a Party-backed actor on
//! the other, alternating strictly, with delivery acks arriving on handover
//! and the exchange concluded by a response-only Close.

use kamiroh::adapter_memory::MemoryNet;
use kamiroh::adapter_memory::testing::block_on;
use kamiroh::app::inbound::{Inbound, process};
use kamiroh::app::parties::CountdownParty;
use kamiroh::app::phone::Phone;
use kamiroh::app::runtime::LocalRuntime;
use kamiroh::domain::actor::{ActorName, Address};
use kamiroh::domain::allowlist::Allowlist;
use kamiroh::domain::endpoint::EndpointId;
use kamiroh::domain::hex::Hex;
use kamiroh::domain::protocol::TurnProgress;
use kamiroh::domain::vocabulary::{Message, Request, RequestId, Response, Turn};
use kamiroh::ports::{Inbox, Transport as _};

fn endpoint(s: &str) -> EndpointId {
    EndpointId::new(Hex::new(s).unwrap())
}

fn name(s: &str) -> ActorName {
    ActorName::new(s).unwrap()
}

fn address(e: &str, n: &str) -> Address {
    Address::new(endpoint(e), name(n))
}

fn request(n: u8) -> Request {
    Request {
        id: RequestId([n; 16]),
        body: vec![n],
    }
}

#[test]
fn a_multi_round_exchange_with_acks_on_every_handover() {
    block_on(async {
        let net = MemoryNet::new();

        // App side: a hand-rolled actor holding a Phone.
        let app = address("aa", "app");
        let mut app_inbox = net.register(app.clone()).unwrap();
        let mut app_list = Allowlist::empty();
        app_list.admit(endpoint("bb"));

        // Far side: the toy runtime hosting a CountdownParty(2) — it will
        // pose two requests of its own before closing.
        let mut runtime = LocalRuntime::new(endpoint("bb"), net.transport(), net.clone());
        let mut party_list = Allowlist::empty();
        party_list.admit(endpoint("aa"));
        runtime
            .install_party(
                name("counter"),
                party_list,
                Box::new(CountdownParty::new(2)),
            )
            .unwrap();
        let counter = address("bb", "counter");

        let mut phone = Phone::converse(app.clone(), counter.clone(), net.transport());

        // Open the exchange.
        phone.open(request(1)).await.unwrap();

        let mut acks = 0;
        let mut rounds = 0;
        let mut concluded = false;
        let mut next_fresh: u8 = 10;

        while !concluded {
            runtime.step(&name("counter")).await.unwrap();
            // Drain what the far side produced for us: an Ack (handover
            // receipt) and then the party's turn.
            loop {
                let delivery = app_inbox.next().await.unwrap();
                match process(&app_list, delivery) {
                    Inbound::AckReceived(_) => {
                        acks += 1;
                        continue;
                    }
                    Inbound::Turn { turn, .. } => {
                        let progress = phone.on_incoming(&turn).unwrap();
                        match (progress, turn) {
                            (TurnProgress::Concluded, Turn::Close { .. }) => {
                                concluded = true;
                            }
                            (
                                TurnProgress::Continuing,
                                Turn::Continue {
                                    request: theirs, ..
                                },
                            ) => {
                                rounds += 1;
                                // Answer their request; keep the exchange
                                // open with a fresh question of our own.
                                let reply = Turn::Continue {
                                    response: Response {
                                        id: theirs.id,
                                        body: theirs.body,
                                    },
                                    request: request(next_fresh),
                                };
                                next_fresh += 1;
                                phone.send_turn(reply).await.unwrap();
                            }
                            other => panic!("unexpected progress/turn: {other:?}"),
                        }
                        break;
                    }
                    other => panic!("unexpected inbound: {other:?}"),
                }
            }
        }

        // CountdownParty(2) poses two requests before closing: two mid-rounds.
        assert_eq!(rounds, 2);
        // Every turn we sent posed a request (Open + 2 Continues) → 3 acks.
        assert_eq!(acks, 3);
        // Alternation left the phone idle: a new exchange may begin.
        phone.open(request(200)).await.unwrap();
    });
}

#[test]
fn the_phone_refuses_out_of_turn_sends() {
    block_on(async {
        let net = MemoryNet::new();
        let app = address("aa", "app");
        let _inbox = net.register(app.clone()).unwrap();
        let peer = address("bb", "counter");
        // Register the peer so sends do not fail at the transport.
        let _peer_inbox = net.register(peer.clone()).unwrap();

        let mut phone = Phone::converse(app, peer, net.transport());
        phone.open(request(1)).await.unwrap();
        // We just spoke; a second turn before their reply is refused locally.
        let err = phone.open(request(2)).await.unwrap_err();
        assert!(matches!(
            err,
            kamiroh::app::phone::PhoneError::Turn(kamiroh::domain::protocol::TurnError::NotOurMove)
        ));
        // And a Close out of nowhere is equally illegal.
        let mut fresh = Phone::converse(
            address("aa", "app2"),
            address("bb", "counter"),
            net.transport(),
        );
        let err = fresh
            .send_turn(Turn::Close {
                response: Response {
                    id: RequestId([9; 16]),
                    body: vec![],
                },
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            kamiroh::app::phone::PhoneError::Turn(kamiroh::domain::protocol::TurnError::NoExchange)
        ));
    });
}

#[test]
fn spawned_echo_party_closes_a_turn_exchange() {
    block_on(async {
        let net = MemoryNet::new();
        let app = address("aa", "app");
        let mut app_inbox = net.register(app.clone()).unwrap();
        let mut app_list = Allowlist::empty();
        app_list.admit(endpoint("bb"));

        let mut runtime = LocalRuntime::new(endpoint("bb"), net.transport(), net.clone());
        let mut list = Allowlist::empty();
        list.admit(endpoint("aa"));
        runtime
            .install(
                name("harness"),
                list,
                kamiroh::app::runtime::ActorKind::Harness,
            )
            .unwrap();

        // Spawn an echo actor via the harness protocol.
        let mut t = net.transport();
        t.send(
            &app,
            &address("bb", "harness"),
            Message::Harness(kamiroh::domain::vocabulary::Harness::Spawn { name: name("echo") }),
        )
        .await
        .unwrap();
        runtime.step(&name("harness")).await.unwrap();
        let _spawned = app_inbox.next().await.unwrap(); // Spawned reply

        // Converse with it in turns: open → (ack, close-echo).
        let echo = address("bb", "echo");
        let mut phone = Phone::converse(app.clone(), echo, net.transport());
        phone.open(request(7)).await.unwrap();
        runtime.step(&name("echo")).await.unwrap();

        let mut saw_ack = false;
        loop {
            let delivery = app_inbox.next().await.unwrap();
            match process(&app_list, delivery) {
                Inbound::AckReceived(ack) => {
                    assert_eq!(ack.id, RequestId([7; 16]));
                    saw_ack = true;
                }
                Inbound::Turn { turn, .. } => {
                    assert_eq!(phone.on_incoming(&turn).unwrap(), TurnProgress::Concluded);
                    let Turn::Close { response } = turn else {
                        panic!("echo should close immediately");
                    };
                    assert_eq!(response.id, RequestId([7; 16]));
                    assert_eq!(response.body, vec![7]);
                    break;
                }
                other => panic!("unexpected inbound: {other:?}"),
            }
        }
        assert!(
            saw_ack,
            "the delivery ack should precede the party's answer"
        );
    });
}
