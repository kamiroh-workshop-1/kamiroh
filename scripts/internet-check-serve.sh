#!/usr/bin/env bash
# internet-check-serve.sh — scoped, self-expiring port-forward for the
# internet check (docs/INTERNET-CHECK.md), macOS server side.
#
# Design: the router mapping is a *lease*, not standing configuration.
#   - `run` couples every lifetime together: open the lease, start the
#     kamiroh server on the fixed port, renew the lease in the background,
#     and on Ctrl-C (or any exit) revoke the lease again. One command is
#     one test window.
#   - The lease has a TTL (default 2h), so even a crash or forgotten
#     terminal fails safe: the router closes the door by itself.
#   - `open`/`close`/`status` exist for manual control and inspection.
#
# Requires libnatpmp's `natpmpc` (brew install libnatpmp) for routers
# speaking NAT-PMP/PCP (most Apple/consumer gear), or falls back to
# miniupnpc's `upnpc` (brew install miniupnpc) for UPnP-only routers. If
# the router speaks neither, use its admin UI and treat this script's
# open/close as your checklist.
#
# usage:
#   scripts/internet-check-serve.sh run   [port] -- <harness_ping serve args>
#   scripts/internet-check-serve.sh open  [port] [ttl-seconds]
#   scripts/internet-check-serve.sh close [port]
#   scripts/internet-check-serve.sh status
#
# example (the full test window, Ctrl-C to end):
#   scripts/internet-check-serve.sh run 4711 -- \
#     --secret 0202... --allow 8a88e3...

set -euo pipefail

CMD=${1:-}; shift || true
PORT=${1:-4711}
TTL=7200

have() { command -v "$1" >/dev/null 2>&1; }

lan_ip() {
  # The mini's primary LAN address (for UPnP, which needs it explicitly).
  ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1
}

map_open() {
  local ttl=$1
  if have natpmpc; then
    natpmpc -a "$PORT" "$PORT" udp "$ttl" | grep -E 'Mapped|public' || true
    echo "lease: UDP $PORT for ${ttl}s (NAT-PMP)"
  elif have upnpc; then
    upnpc -e "kamiroh internet check" -a "$(lan_ip)" "$PORT" "$PORT" UDP "$ttl" \
      | grep -iE 'external|mapped' || true
    echo "lease: UDP $PORT for ${ttl}s (UPnP)"
  else
    echo "neither natpmpc nor upnpc found (brew install libnatpmp or miniupnpc)." >&2
    echo "open UDP $PORT on the router UI manually — and close it after." >&2
    return 1
  fi
}

map_close() {
  if have natpmpc; then
    natpmpc -a "$PORT" "$PORT" udp 0 >/dev/null 2>&1 || true
    echo "lease revoked: UDP $PORT (NAT-PMP)"
  elif have upnpc; then
    upnpc -d "$PORT" UDP >/dev/null 2>&1 || true
    echo "lease revoked: UDP $PORT (UPnP)"
  fi
}

public_ip() {
  if have natpmpc; then
    natpmpc 2>/dev/null | grep -oE '([0-9]{1,3}\.){3}[0-9]{1,3}' | head -1
  elif have upnpc; then
    upnpc -s 2>/dev/null | grep -i 'ExternalIPAddress' | grep -oE '([0-9]{1,3}\.){3}[0-9]{1,3}'
  fi
}

case "$CMD" in
  open)
    TTL=${2:-$TTL}
    map_open "$TTL"
    echo "public address (give the checker): $(public_ip):$PORT"
    ;;
  close)
    map_close
    ;;
  status)
    if have upnpc; then upnpc -l 2>/dev/null | grep -i "UDP.*$PORT" || true; fi
    echo "public ip: $(public_ip)"
    echo "(NAT-PMP leases are not enumerable; rely on TTL + close.)"
    ;;
  run)
    shift || true                       # drop port
    [ "${1:-}" = "--" ] && shift        # drop separator
    BIN=target/release/examples/harness_ping
    [ -x "$BIN" ] || { echo "build first: cargo build --release --example harness_ping -p kamiroh-adapter-iroh" >&2; exit 1; }
    map_open "$TTL"
    echo "public address (give the checker): $(public_ip):$PORT"
    # Revoke the lease however this window ends.
    trap 'map_close; kill "${RENEW_PID:-0}" 2>/dev/null || true' EXIT INT TERM
    # Renew at half-TTL so long windows survive.
    ( while sleep $((TTL / 2)); do map_open "$TTL" >/dev/null 2>&1 || true; done ) &
    RENEW_PID=$!
    echo "serving on UDP $PORT — Ctrl-C ends the window and revokes the lease."
    "$BIN" serve --port "$PORT" "$@"
    ;;
  *)
    grep '^# usage' -A 8 "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
