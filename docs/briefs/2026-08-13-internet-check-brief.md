# Brief: the internet check — first live validation of the n0 profile

*2026-08-13. Runbook: `docs/INTERNET-CHECK.md`. This was `NetProfile::N0`'s
(decision 21) first contact with reality — CI cannot reach n0's
infrastructure, so nothing before these runs had ever exercised the
profile end to end. Two networks tested; two passes; the direct path won
both times.*

## Leg 1 (phone hotspot): CHECK PASSED, direct path selected

Server on a Mac mini on its home network, behind multiple NAT layers.
Checker on a laptop connected through a phone hotspot. Both sides ran
master `1391132`'s `harness_ping` under `--net n0`. No port-forwards, no
addresses exchanged, no configuration of any kind touching the path: the
checker dialed by endpoint id alone.

- `PONG 752.9ms` — first contact, so this includes the n0 address lookup,
  relay rendezvous, and QUIC handshake, not just the round trip.
- `SPAWNED echo-incus`, `TURN OK (ack seen: true)` — remote spawn and a
  full turn exchange with the delivery ack, over the real path.
- `PATHS` listed two paths: the n0 relay (`use1-1.relay.n0.iroh.link`)
  and a direct ip path — **with the direct path selected**. Hole-punching
  won; the relay stood by as fallback. This is the stronger of the two
  possible passes.

Specific addresses and network topology are deliberately not recorded.

## Method notes: three runs, two instructive failures

1. **Run 1 passed, but confounded.** An overlay VPN was active on both
   machines, and the selected "direct" path was the overlay's virtual
   address — the overlay's own NAT traversal, not iroh's. Lesson: overlay
   interfaces are published into the n0 address record like any other
   interface; turn overlays off for path-validation runs, or the
   measurement measures the overlay.
2. **Run 2 failed loudly, also instructive.** With the overlay off on one
   side but the server not restarted, the checker dialed the stale overlay
   address from the still-published record and *something else* answered,
   rejecting the handshake with TLS alert 120 ("peer doesn't support any
   known protocol"). That alert arrives before the dialer verifies the
   responder's identity — so a handshake rejection is not necessarily the
   peer speaking. Lesson: after changing interfaces, restart the server so
   it republishes a fresh record.
3. **Run 3, clean.** Overlays off on both sides, server restarted, fresh
   record: the pass described above.

## Leg 2 (café wifi): CHECK PASSED, direct path again

Same server, same laptop, third network kind — a café's wifi, no
coordination with the server side beyond it being up. First-contact
`PONG 773.3ms`, then the full sequence through `CHECK PASSED`. `PATHS`
again listed the relay plus a direct ip path, with the direct path
selected: hole-punching succeeded from this network too. Addresses
again unrecorded.

This leg also upgraded the key hygiene: because the server had to run
unattended during the outing, both sides used **one-off identities**
(fresh random secrets, ids exchanged out of band) instead of the repo's
world-readable demo secrets. That closes both unattended risks at once:
the allowlist admits exactly one id whose secret nobody else holds, and
the published address record is unfetchable — n0 lookups are by id, and
the one-off id appears nowhere public. A self-stop timer (`kill` after a
fixed window) plus `caffeinate -i` bounded the window and kept the
server awake through it.

## Hygiene

The demo secrets are world-readable in this repo, so servers using them
ran only attended, during test windows, and were stopped after each run;
the published address record ceases being served when the server stops.
For unattended windows, use one-off identities as in leg 2 — this should
be the default posture for any future run longer than a coffee.

## Pending

Nothing owed. Further networks are free data points whenever convenient:
same laptop steps, record network kind, first-contact RTT, and
direct-vs-relay from `PATHS`.
