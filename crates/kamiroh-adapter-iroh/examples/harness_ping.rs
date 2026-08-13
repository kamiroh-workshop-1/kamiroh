//! The cross-container check binary (`docs/INCUS-CHECK.md`).
//!
//! Two modes, one per container:
//!
//! - `serve  --secret <64-hex> --allow <peer endpoint id, 64-hex>`
//!   Binds an endpoint, hosts a Kameo runtime with a harness actor admitting
//!   the given peer, prints its own `ID` and `ADDR` lines, then waits.
//! - `check  --secret <64-hex> --peer-id <64-hex> --peer-ip <ip:port>`
//!   Binds an endpoint, introduces the peer statically, then runs the
//!   spike's standard proof across the wire: harness ping → remote spawn →
//!   a turn exchange with the spawned echo party. Exits 0 on success.
//!
//! Demo-grade key handling on purpose: secrets arrive as hex CLI args so the
//! two sides can know each other's endpoint ids upfront (static
//! introduction, decision 19). Do not imitate for anything real.

use std::net::SocketAddr;
use std::process::exit;
use std::str::FromStr;
use std::time::{Duration, Instant};

use iroh::{EndpointAddr, TransportAddr};
use tokio::time::timeout;

use kamiroh_adapter_iroh::{IrohInbox, IrohNet, NetProfile};
use kamiroh_adapter_kameo::KameoRuntime;
use kamiroh_app::inbound::{Inbound, process};
use kamiroh_app::phone::Phone;
use kamiroh_app::runtime::ActorKind;
use kamiroh_domain::actor::{ActorName, Address};
use kamiroh_domain::allowlist::Allowlist;
use kamiroh_domain::endpoint::EndpointId;
use kamiroh_domain::hex::Hex;
use kamiroh_domain::protocol::TurnProgress;
use kamiroh_domain::secret::Secret;
use kamiroh_domain::vocabulary::{Harness, Message, Request, RequestId, Turn};
use kamiroh_ports::{Inbox, Registry, Transport};

const WAIT: Duration = Duration::from_secs(15);

fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("not a hex string: {s}"));
    }
    Ok((0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect())
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn name(s: &str) -> ActorName {
    ActorName::new(s).unwrap()
}

fn usage() -> ! {
    eprintln!(
        "usage:\n  harness_ping serve --secret <64-hex> --allow <peer-id-64-hex> [--net hermetic|n0] [--port <n>]\n  \
         harness_ping check --secret <64-hex> --peer-id <64-hex> [--peer-ip <ip:port>] [--net hermetic|n0]\n\
         --peer-ip is required under --net hermetic; under --net n0 the id alone suffices\n\
         (n0 = relays + address lookup: dial by id, NATs handled by iroh)\n\
         --port fixes the listen port (relay-less pre-arranged dialability)"
    );
    exit(2);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).cloned().unwrap_or_default();
    let secret_hex = arg(&args, "--secret").unwrap_or_else(|| usage());
    let secret_bytes = hex_to_bytes(&secret_hex).unwrap_or_else(|e| {
        eprintln!("--secret: {e}");
        exit(2)
    });
    let secret = Secret::new(secret_bytes);
    let profile = match arg(&args, "--net").as_deref() {
        None | Some("hermetic") => NetProfile::Hermetic,
        Some("n0") => NetProfile::N0,
        Some(other) => {
            eprintln!("--net must be hermetic or n0, got {other}");
            exit(2)
        }
    };

    let port: Option<u16> = arg(&args, "--port").map(|p| {
        p.parse().unwrap_or_else(|_| {
            eprintln!("--port must be a number");
            exit(2)
        })
    });
    let net = match (profile, port) {
        (NetProfile::Hermetic, port) => IrohNet::bind_on(&secret, port).await,
        (profile, None) => IrohNet::bind_with(&secret, profile).await,
        (_, Some(_)) => {
            eprintln!("--port only applies under --net hermetic (n0 dials by id)");
            exit(2)
        }
    }
    .unwrap_or_else(|e| {
        eprintln!("bind failed: {e}");
        exit(1)
    });
    println!("ID {}", net.endpoint_id());
    let our_addr = net.addr().await.expect("no dialable address");
    for a in &our_addr.addrs {
        println!("ADDR {a}");
    }

    match mode.as_str() {
        "serve" => serve(net, &arg(&args, "--allow").unwrap_or_else(|| usage())).await,
        "check" => {
            let peer_ip = arg(&args, "--peer-ip");
            if peer_ip.is_none() && profile == NetProfile::Hermetic {
                eprintln!("--peer-ip is required under --net hermetic");
                usage()
            }
            check(
                net,
                &arg(&args, "--peer-id").unwrap_or_else(|| usage()),
                peer_ip.as_deref(),
            )
            .await
        }
        _ => usage(),
    }
}

async fn serve(net: IrohNet, allow_hex: &str) {
    let peer = EndpointId::new(Hex::new(allow_hex).expect("--allow must be hex"));
    let mut allowlist = Allowlist::empty();
    allowlist.admit(peer);

    let runtime = KameoRuntime::new(net.endpoint_id().clone(), net.transport(), net.clone());
    runtime
        .install(name("harness"), allowlist, ActorKind::Harness)
        .expect("install harness");
    println!("READY");
    std::future::pending::<()>().await;
}

async fn check(net: IrohNet, peer_id_hex: &str, peer_ip: Option<&str>) {
    // Introduction: with an explicit ip:port, static (hermetic). By id
    // alone (n0), address lookup + relays find the path — no exterior
    // path-figuring required.
    let peer_iroh_id =
        iroh::EndpointId::from_str(peer_id_hex).expect("--peer-id must be a 64-hex endpoint id");
    let peer = match peer_ip {
        Some(ip) => {
            let sock = SocketAddr::from_str(ip).expect("--peer-ip must be ip:port");
            net.add_peer(EndpointAddr::from_parts(
                peer_iroh_id,
                [TransportAddr::Ip(sock)],
            ))
        }
        None => net.add_peer(EndpointAddr::new(peer_iroh_id)),
    };

    let mut allowlist = Allowlist::empty();
    allowlist.admit(peer.clone());

    let controller = Address::new(net.endpoint_id().clone(), name("controller"));
    let mut registry = net.clone();
    let mut inbox = registry.bind(&controller).expect("bind controller");
    let mut transport = net.transport();
    let harness = Address::new(peer.clone(), name("harness"));

    // -- 1. Ping across the container boundary --------------------------
    let t0 = Instant::now();
    transport
        .send(&controller, &harness, Message::Harness(Harness::Ping))
        .await
        .expect("send ping");
    expect_harness(&mut inbox, &allowlist, |h| *h == Harness::Pong).await;
    println!("PONG {:?}", t0.elapsed());

    // -- 2. Spawn an echo party remotely ---------------------------------
    transport
        .send(
            &controller,
            &harness,
            Message::Harness(Harness::Spawn {
                name: name("echo-incus"),
            }),
        )
        .await
        .expect("send spawn");
    expect_harness(&mut inbox, &allowlist, |h| {
        *h == Harness::Spawned {
            name: name("echo-incus"),
        }
    })
    .await;
    println!("SPAWNED echo-incus");

    // -- 3. A turn exchange with it --------------------------------------
    let echo = Address::new(peer.clone(), name("echo-incus"));
    let mut phone = Phone::converse(controller.clone(), echo, net.transport());
    let body = b"hello from the other container".to_vec();
    phone
        .open(Request {
            id: RequestId([7; 16]),
            body: body.clone(),
        })
        .await
        .expect("open turn exchange");
    let mut saw_ack = false;
    loop {
        let delivery = timeout(WAIT, inbox.next())
            .await
            .expect("timed out awaiting turn reply")
            .expect("inbox closed");
        match process(&allowlist, delivery) {
            Inbound::AckReceived(_) => saw_ack = true,
            Inbound::Turn { turn, .. } => {
                assert_eq!(
                    phone.on_incoming(&turn).expect("legal turn"),
                    TurnProgress::Concluded
                );
                let Turn::Close { response } = turn else {
                    panic!("echo should close immediately");
                };
                assert_eq!(response.body, body, "echo must return the body");
                break;
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
    }
    println!("TURN OK (ack seen: {saw_ack})");
    if let Some(paths) = net.paths_to(&peer).await {
        println!("PATHS {paths}");
    }
    println!("CHECK PASSED");
}

async fn expect_harness(
    inbox: &mut IrohInbox,
    allowlist: &Allowlist,
    want: impl Fn(&Harness) -> bool,
) {
    loop {
        let delivery = timeout(WAIT, inbox.next())
            .await
            .expect("timed out awaiting harness reply")
            .expect("inbox closed");
        match process(allowlist, delivery) {
            Inbound::AckReceived(_) => continue,
            Inbound::Harness { harness, .. } if want(&harness) => return,
            other => panic!("unexpected inbound: {other:?}"),
        }
    }
}
