# The internet check: kamiroh across the real internet

*Runbook for the first true internet-path validation: a laptop on a phone
hotspot (behind carrier-grade NAT) conversing with a Mac mini at home
(behind a home router, made dialable by one port-forward). A café variant
follows free of charge: same steps, different unknown NAT.*

## What this validates — and what it doesn't

Validates: kamiroh conversations over the actual internet — real latency,
real packet loss, CGNAT egress from the laptop side, and the learned-peer
property (the mini needs **no** peer-book entry for its callers; replies
ride the connection the laptop opened). Does **not** validate hole-punching:
one side is deliberately dialable via port-forward. Double-blind traversal
stays with the deferred relay/discovery work — this check is its
prerequisite rung.

## One-time setup (Mac mini side)

1. Pick a UDP port, e.g. `4711`. On the home router, forward **UDP** 4711 →
   the mini's LAN IP, port 4711.
2. Allow the binary through macOS's firewall if prompted on first run.
3. Note the home network's public IP (e.g. from the router's status page).
   If the ISP uses CGNAT for the *home* connection too (public IP on the
   router differs from what whatismyip-style services report), the forward
   will not be reachable — that finding itself is worth recording.

## Steps

```sh
# Mini (server) — fixed port so the forward stays valid across runs:
cd kamiroh && cargo build --release --example harness_ping -p kamiroh-adapter-iroh
./target/release/examples/harness_ping serve \
  --secret 0202020202020202020202020202020202020202020202020202020202020202 \
  --allow 8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c \
  --port 4711
# prints ID 8139...b394 and ADDR lines showing its LAN view — that's
# expected; the caller uses the PUBLIC address instead.

# Laptop (checker), on the phone hotspot — clone, build, dial the public IP:
git clone --depth 1 https://github.com/kamiroh-workshop-1/kamiroh.git
cd kamiroh && cargo build --release --example harness_ping -p kamiroh-adapter-iroh
./target/release/examples/harness_ping check \
  --secret 0101010101010101010101010101010101010101010101010101010101010101 \
  --peer-id 8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394 \
  --peer-ip <home-public-ip>:4711
```

Success: `PONG <rtt>` (expect tens of ms via cellular), `SPAWNED echo-incus`,
`TURN OK (ack seen: true)`, `CHECK PASSED`, exit 0.

## Café variant

Identical laptop steps from any other network. Each new network exercises a
different NAT's outbound behavior — record each as its own line in the brief:
network kind, RTT, pass/fail.

## Recording

Add results to a brief under `docs/briefs/` (environment, network kinds,
RTTs, any friction). Note for the security-minded: the demo secrets are
world-readable in this repo, so anyone could dial the forwarded port and be
*admitted by the harness* while the demo allowlist is in place — close the
forward or stop the server when not testing, and treat the whole setup as
demo-grade by construction.
