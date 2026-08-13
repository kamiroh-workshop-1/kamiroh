# The Incus check: kamiroh across container boundaries

*Runbook for the spike's deployment-shape validation: two Incus containers,
one kamiroh endpoint each, the standard proof (harness ping → remote spawn →
turn exchange) across the container boundary. Uses
`crates/kamiroh-adapter-iroh/examples/harness_ping.rs`, rehearsed
end-to-end as two processes before this runbook was written.*

## What this validates — and what it doesn't

Validates: endpoints running inside containers, key-based identity, static
peer introduction, and the full protocol stack over a real network path
between separate machines-as-far-as-userspace-knows. Does **not** validate
NAT hole-punching or relay fallback — both containers sit on the same host
bridge with direct reachability, and the endpoints run relay-disabled
(decision 19). That harder test needs genuinely separated networks; deferred
with the relay-policy work.

## Fixed demo identities

The check uses fixed secrets so both sides know each other's endpoint ids
upfront (demo-grade on purpose; never imitate for real deployments):

- Side A (checker): secret `01`×32 →
  id `8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c`
- Side B (server):  secret `02`×32 →
  id `8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394`

## Steps (on the Incus host)

```sh
# 1. Two containers
incus launch images:debian/13 kamiroh-a
incus launch images:debian/13 kamiroh-b

# 2. Toolchain + clone + build in each (a few minutes each)
for c in kamiroh-a kamiroh-b; do
  incus exec $c -- bash -c '
    apt-get update -qq && apt-get install -y -qq curl git build-essential
    curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q
    . "$HOME/.cargo/env"
    git clone --depth 1 https://github.com/kamiroh-workshop-1/kamiroh.git
    cd kamiroh && cargo build --release --example harness_ping -p kamiroh-adapter-iroh
  '
done

# 3. Start the server in B (stays in the foreground; note its ADDR line)
incus exec kamiroh-b -- bash -c '
  cd kamiroh && ./target/release/examples/harness_ping serve \
    --secret 0202020202020202020202020202020202020202020202020202020202020202 \
    --allow 8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c
'
# It prints:  ID 8139...b394,  ADDR ip:<container-B-ip>:<port>,  READY
# Use the ip:port from the ADDR line matching B's bridge address
# (cross-check with: incus list kamiroh-b -c 4)

# 4. Run the check from A (second shell)
incus exec kamiroh-a -- bash -c '
  cd kamiroh && ./target/release/examples/harness_ping check \
    --secret 0101010101010101010101010101010101010101010101010101010101010101 \
    --peer-id 8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394 \
    --peer-ip <container-B-ip>:<port>
'

# 5. Cleanup
incus delete -f kamiroh-a kamiroh-b
```

## Success criteria

The checker prints, in order: `PONG <rtt>`, `SPAWNED echo-incus`,
`TURN OK (ack seen: true)`, `CHECK PASSED`, and exits 0. The `ack seen: true`
matters: it confirms the delivery receipt beat the party's answer across the
wire, the layering decision 4 promises. Any timeout (15 s per step) exits
nonzero with a message saying which step starved.

Note only B's address is ever configured: A is introduced to B statically,
while B learns A from the inbound connection itself (replies ride the
connection the request arrived on — see the symmetric-reader note in the
adapter). Record results in a brief under `docs/briefs/` as usual.
