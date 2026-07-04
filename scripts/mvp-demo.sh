#!/usr/bin/env bash
#
# mvp-demo.sh — run the beaterOS Minimum Viable proof end to end, locally.
#
# This drives the real `beaterosctl` through the full contract loop from
# `final.md` §24: create a session, issue a scoped capability grant, execute an
# action in the sandbox, verify the hash-chained journal, and render a trace
# with receipts. It also proves the two fail-closed paths: an action with no
# grant, and an action whose grant does not cover the target path, must both be
# refused with no side effect.
#
# It is both a human-readable demo (it prints each step) and a smoke gate: it
# asserts every expected outcome and exits non-zero on any failure, so it can be
# wired into CI or run by hand. No network, no external services; macOS and
# Linux only need bash + /bin/sh.
#
# Usage:  scripts/mvp-demo.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/beateros-mvp-demo.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT
# Canonicalize the work dir so the granted path-prefix matches the sandbox's
# realpath'd cwd. On macOS $TMPDIR is under /var, a symlink to /private/var, and
# grant prefixes are not yet canonicalized at issue time (issue #102); without
# this a legitimately-granted action would be wrongly refused as out-of-scope.
WORK="$(cd "$WORK" && pwd -P)"

export BEATEROS_HOME="$WORK/home"
WS="$WORK/ws"          # the granted workspace
OTHER="$WORK/elsewhere" # a path the demo grant will NOT cover
mkdir -p "$WS" "$OTHER"

pass=0
fail=0
ok()   { echo "  ok   — $1"; pass=$((pass + 1)); }
bad()  { echo "  FAIL — $1"; fail=$((fail + 1)); }

step() {
  echo
  echo "==> $1"
}

echo "beaterOS MVP proof (final.md §24) — store: $BEATEROS_HOME"

step "Build beaterosctl"
cargo build -q -p beaterosctl
BCTL="$ROOT/target/debug/beaterosctl"

step "1. Create an agent session from a goal"
"$BCTL" session create --agent agent:demo --workspace ws-demo \
  --goal "Read an IoT sensor into the granted workspace" --session sess-demo
ok "session created"

step "2. Issue a scoped capability grant (workspace read/write/execute)"
"$BCTL" grant issue --session sess-demo --resource-kind file_path --resource-id "$WS" \
  --actions read,write,execute --path-prefix "$WS" --max-risk high \
  --reason "scoped demo workspace" >"$WORK/grant.out" 2>&1
cat "$WORK/grant.out"
GRANT="$(sed -n 's/^issued grant \([0-9a-f-]*\).*/\1/p' "$WORK/grant.out")"
[ -n "$GRANT" ] && ok "grant issued ($GRANT)" || bad "no grant id parsed"

step "3. Execute an action in the sandbox (writes a sensor reading)"
"$BCTL" action execute --session sess-demo --tool tool:shell --command /bin/sh \
  --arg -c --arg 'printf 22.5 > sensor.txt' --cwd "$WS" --grants "$GRANT" \
  --risk low --side-effects local_write --idempotency-key idem-read >"$WORK/exec.out" 2>&1
cat "$WORK/exec.out"
grep -q 'decision:.*Allowed' "$WORK/exec.out" && ok "action admitted by the grant" || bad "action was not Allowed"
grep -q 'receipt:' "$WORK/exec.out" && ok "receipt emitted for the side effect" || bad "no receipt emitted"
[ "$(cat "$WS/sensor.txt" 2>/dev/null || true)" = "22.5" ] && ok "observed effect: sensor.txt written in the workspace" || bad "sensor.txt not written"

step "4. Verify the hash-chained journal and receipts"
"$BCTL" journal verify --session sess-demo >"$WORK/verify.out" 2>&1
cat "$WORK/verify.out"
grep -q 'journal OK' "$WORK/verify.out" && ok "journal + receipt chains verify" || bad "journal did not verify"

step "5a. Fail closed: an action with NO grant is refused"
if "$BCTL" action execute --session sess-demo --tool tool:shell --command /bin/sh \
  --arg -c --arg 'printf leak > exfil.txt' --cwd "$WS" --grants "" \
  --risk low --idempotency-key idem-nogrant >"$WORK/nogrant.out" 2>&1; then
  bad "no-grant action was NOT refused"
else
  ok "no-grant action refused: $(head -1 "$WORK/nogrant.out")"
fi
[ -f "$WS/exfil.txt" ] && bad "exfil.txt leaked" || ok "no side effect from the refused action"

step "5b. Fail closed: a grant that does not cover the target path is refused"
"$BCTL" grant issue --session sess-demo --resource-kind file_path --resource-id "$OTHER" \
  --actions read,write,execute --path-prefix "$OTHER" --max-risk high \
  --reason "narrow grant covering elsewhere" >"$WORK/grant2.out" 2>&1
NARROW="$(sed -n 's/^issued grant \([0-9a-f-]*\).*/\1/p' "$WORK/grant2.out")"
if "$BCTL" action execute --session sess-demo --tool tool:shell --command /bin/sh \
  --arg -c --arg 'printf escaped > outside.txt' --cwd "$WS" --grants "$NARROW" \
  --risk low --side-effects local_write --idempotency-key idem-escape >"$WORK/escape.out" 2>&1; then
  bad "out-of-scope action was NOT refused"
else
  ok "out-of-scope action refused: $(head -1 "$WORK/escape.out")"
fi
[ -f "$WS/outside.txt" ] && bad "workspace escape leaked outside.txt" || ok "no escape: nothing written outside the granted prefix"

step "6. Show the trace (grants, actions, decisions, receipts)"
"$BCTL" trace show --session sess-demo

echo
echo "===================================================================="
echo "MVP proof: $pass checks passed, $fail failed."
if [ "$fail" -ne 0 ]; then
  echo "RESULT: FAIL"
  exit 1
fi
echo "RESULT: PASS — beaterOS runs the full §24 contract loop locally."
