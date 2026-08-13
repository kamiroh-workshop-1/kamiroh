# The internet check: kamiroh across the real internet

*Runbook for the first true internet-path validation: a laptop on a phone
hotspot conversing with a Mac mini at home — both behind however many NAT
layers they happen to be behind. No port-forwards, no path configuration:
under the `n0` net profile (`ARCHITECTURE.md`, decision 21), Iroh's own
machinery — n0's relay fleet plus address publishing/lookup — finds the
path. A café variant follows free of charge: same laptop steps, different
unknown network.*

## What this validates

The real thing, at last: dial-by-id across the public internet, address
records resolved through n0's lookup service, rendezvous through a relay,
**hole-punching** attempted for a direct path with relay fallback if the
NATs win, and the full protocol stack (harness ping → remote spawn → turn
exchange, ack before answer) riding whatever path results. The check's
final `PATHS` line reports which: a direct path listed alongside the relay
means hole-punching succeeded; relay-only means the NATs were stubborn and
the relay carried the conversation — both are passes, and *which one* is
the finding worth recording per network.

This run depends on n0's public infrastructure being reachable — that
dependency is the deliberate choice of decision 21, and the hermetic
profile remains the default everywhere else.

## Steps

```sh
# Mini (server) — no port, no forward, no lease:
cd kamiroh && cargo build --release --example harness_ping -p kamiroh-adapter-iroh
./target/release/examples/harness_ping serve --net n0 \
  --secret 0202020202020202020202020202020202020202020202020202020202020202 \
  --allow 8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c
# prints ID 8139...b394 (and ADDR lines, informational only), then READY.
# Give it ~10s after READY for its address record to publish.

# Laptop (checker), on the hotspot — dial by id alone:
git clone --depth 1 https://github.com/kamiroh-workshop-1/kamiroh.git
cd kamiroh && cargo build --release --example harness_ping -p kamiroh-adapter-iroh
./target/release/examples/harness_ping check --net n0 \
  --secret 0101010101010101010101010101010101010101010101010101010101010101 \
  --peer-id 8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394
```

Success: `PONG <rtt>`, `SPAWNED echo-incus`, `TURN OK (ack seen: true)`,
`PATHS <the path list>`, `CHECK PASSED`, exit 0. First contact may take a
few seconds longer than later ones (lookup + rendezvous); each step allows
15 s before failing loudly.

## Café variant

Identical laptop steps from any other network, no coordination with home
needed (the mini's record is already published). Record per network: kind,
RTT, and whether `PATHS` shows a direct path (hole-punched) or relay-only.

## Recording and hygiene

Results go in a brief under `docs/briefs/` as usual. Security notes, both
sides of the same coin: the demo secrets are world-readable in this repo,
so while the server runs, anyone on the internet could present themselves
as the demo checker id and be admitted by the harness — run the server only
during test windows. And under the n0 profile the endpoint *publishes* a
signed address record to n0's public lookup service — inherent to
discoverability, worth knowing it's public, and it stops when the server
stops.

For any window you can't attend — the server running while you're out
testing from elsewhere — don't use the demo secrets at all. Generate
**one-off identities** instead: a fresh random secret per side
(`openssl rand -hex 32`), learn each side's id from the `ID` line of a
quick hermetic serve, and exchange only the ids out of band. Both risks
close at once: the allowlist admits exactly one id whose secret nobody
else holds, and the published record becomes unfetchable in practice —
n0 lookups are keyed by id, and a one-off id appears nowhere public. Add
a self-stop timer on the server and the window bounds itself. This is
the default posture for any run longer than a coffee; the café leg of
[the first brief](briefs/2026-08-13-internet-check-brief.md) ran this way.

## The relay-less variant (single-NAT setups, no n0 dependency)

Where the server side sits behind exactly one NAT that its operator
controls, the check also works without n0's infrastructure: run the server
under the default hermetic profile on a fixed port (`--port`, no `--net`
flag) and make that port dialable — `scripts/internet-check-serve.sh`
wraps the whole window as a time-limited NAT-PMP/UPnP *lease* (open →
serve → renew → revoke on Ctrl-C, TTL as the forget-me fail-safe). The
checker then dials `--peer-ip <public-ip>:<port>` explicitly. This
variant cannot help multi-NAT or CGNAT'd servers — that boundary is
precisely why the n0 profile above is the primary flow (decision 21).
