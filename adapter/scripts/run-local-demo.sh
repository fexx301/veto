#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SOLANA_TOOLS="${SOLANA_TOOLS:-$HOME/.local/share/solana/install/active_release/bin}"
BUILD_SBF="$SOLANA_TOOLS/cargo-build-sbf"
SOLANA="$SOLANA_TOOLS/solana"
KEYGEN="$SOLANA_TOOLS/solana-keygen"
VALIDATOR="$SOLANA_TOOLS/solana-test-validator"
RUN_MODE="regression"
if (( $# > 0 )); then
  RUN_MODE="$1"
fi
if [[ "$RUN_MODE" == "--operator" && -z "${OPERATOR_PUBKEY:-}" ]]; then
  echo "--operator requires OPERATOR_PUBKEY for a user-held test wallet" >&2
  echo "example: OPERATOR_PUBKEY=<wallet-address> ./adapter/scripts/run-local-demo.sh --operator" >&2
  exit 1
fi
if [[ "$RUN_MODE" == "--operator" || "$RUN_MODE" == "--operator-smoke" ]]; then
  RUN_LOG_MODE="operator"
elif [[ "$RUN_MODE" == "--external-operator-smoke" ]]; then
  RUN_LOG_MODE="external-operator-smoke"
elif [[ "$RUN_MODE" == "--external-operator-browser" ]]; then
  RUN_LOG_MODE="external-operator-browser"
elif [[ "$RUN_MODE" == "--payment" ]]; then
  RUN_LOG_MODE="payment"
elif [[ "$RUN_MODE" == "--route" ]]; then
  RUN_LOG_MODE="route"
else
  RUN_LOG_MODE="regression"
fi
RPC_URL="http://127.0.0.1:8899"
# Refuse to run if another console or validator already holds the ports;
# otherwise browser and HTTP checks would talk to that other process.
# VETO_PORT moves the console off 4173 if another app is using it.
VETO_PORT="${VETO_PORT:-4173}"
for port in "$VETO_PORT" 8899; do
  if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "port $port is already in use; stop the other Veto console or validator first" >&2
    exit 1
  fi
done
WORK="$(mktemp -d /tmp/veto-demo.XXXXXX)"
# Untracked; copy logs worth keeping into verification-logs/ deliberately.
OUTPUT="$ROOT/verification-logs/runs"
RUN_LOG="$OUTPUT/$RUN_LOG_MODE-demo-run.log"
VALIDATOR_LOG="$OUTPUT/$RUN_LOG_MODE-demo-validator.log"
VALIDATOR_PID=""
SERVER_PID=""

cleanup() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [[ -n "$VALIDATOR_PID" ]] && kill -0 "$VALIDATOR_PID" 2>/dev/null; then
    kill "$VALIDATOR_PID" 2>/dev/null || true
    wait "$VALIDATOR_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

for tool in cargo "$BUILD_SBF" "$SOLANA" "$KEYGEN" "$VALIDATOR"; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "missing required tool: $tool" >&2
    exit 1
  fi
done

mkdir -p "$OUTPUT"
cd "$ROOT"
exec > >(tee "$RUN_LOG") 2>&1

echo "Veto local demo"
echo "Veto commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)"
SOLANA_VERSION="$("$SOLANA" --version)"
SBF_VERSION="$("$BUILD_SBF" --version)"
RUST_VERSION="$(rustc --version)"
printf '%s\n%s\n%s\n' "$SOLANA_VERSION" "$SBF_VERSION" "$RUST_VERSION"
[[ "$SOLANA_VERSION" == *"4.2.1"* ]] || { echo "expected Solana CLI 4.2.1" >&2; exit 1; }
[[ "$SBF_VERSION" == *"cargo-build-sbf 4.1.0"* && "$SBF_VERSION" == *"platform-tools v1.54"* && "$SBF_VERSION" == *"rustc 1.89.0"* ]] || {
  echo "expected cargo-build-sbf 4.1.0, platform-tools v1.54, and SBF rustc 1.89.0" >&2
  exit 1
}
[[ "$RUST_VERSION" == *"1.91.0"* ]] || { echo "expected repository Rust toolchain 1.91.0" >&2; exit 1; }

cargo test --locked --manifest-path adapter/Cargo.toml --features client --test real_swig_payload
"$BUILD_SBF" --manifest-path adapter/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
"$BUILD_SBF" --manifest-path adapter/fixtures/relay/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
RELAY_SO="adapter/fixtures/relay/target/deploy/veto_relay.so"
"$BUILD_SBF" --manifest-path adapter/fixtures/target-program/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
# Merchant-pay v1 (reviewed) and v2 (malicious `drain` build) share one source.
PAY_DEPLOY="adapter/fixtures/merchant-pay/target/deploy"
"$BUILD_SBF" --manifest-path adapter/fixtures/merchant-pay/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
cp "$PAY_DEPLOY/veto_merchant_pay.so" "$PAY_DEPLOY/merchant_pay_v1.so"
"$BUILD_SBF" --manifest-path adapter/fixtures/merchant-pay/Cargo.toml --tools-version v1.54 --arch v3 --features drain -- --locked
cp "$PAY_DEPLOY/veto_merchant_pay.so" "$PAY_DEPLOY/merchant_pay_v2.so"
"$BUILD_SBF" --manifest-path adapter/fixtures/payment-router/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
ROUTER_SO="adapter/fixtures/payment-router/target/deploy/veto_payment_router.so"
"$BUILD_SBF" --manifest-path adapter/fixtures/sneaky-cpi/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
SNEAKY_SO="adapter/fixtures/sneaky-cpi/target/deploy/veto_sneaky_cpi.so"
cargo build --locked --manifest-path adapter/Cargo.toml --features client --bin local_swig_flow
cargo build --locked --manifest-path adapter/Cargo.toml --features client --bin operator_server
cargo build --locked --manifest-path adapter/Cargo.toml --features client --bin external_operator_signer
cargo build --locked --manifest-path adapter/Cargo.toml --features client --bin payment_flow
cargo build --locked --manifest-path adapter/Cargo.toml --features client --bin route_flow

shasum -a 256 \
  adapter/target/deploy/veto_swig_gate.so \
  "$RELAY_SO" \
  adapter/fixtures/target-program/target/deploy/veto_demo_target.so \
  "$PAY_DEPLOY/merchant_pay_v1.so" \
  "$PAY_DEPLOY/merchant_pay_v2.so" \
  "$ROUTER_SO" \
  "$SNEAKY_SO"

"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/human.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/agent.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/operator.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/gate.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/target.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/protocol.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/pay.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/router.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/sneaky.json"
"$KEYGEN" new --no-bip39-passphrase --silent --force --outfile "$WORK/relay.json"

HUMAN_ID="$("$KEYGEN" pubkey "$WORK/human.json")"
AGENT_ID="$("$KEYGEN" pubkey "$WORK/agent.json")"
OPERATOR_ID="$("$KEYGEN" pubkey "$WORK/operator.json")"
GATE_ID="$("$KEYGEN" pubkey "$WORK/gate.json")"
TARGET_ID="$("$KEYGEN" pubkey "$WORK/target.json")"
PAY_ID="$("$KEYGEN" pubkey "$WORK/pay.json")"
ROUTER_ID="$("$KEYGEN" pubkey "$WORK/router.json")"
SNEAKY_ID="$("$KEYGEN" pubkey "$WORK/sneaky.json")"
SWIG_ID="swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB"
RELAY_ID="$("$KEYGEN" pubkey "$WORK/relay.json")"
TARGET_SO="$ROOT/adapter/fixtures/target-program/target/deploy/veto_demo_target.so"

# By default the validator runs the official Swig program cloned from devnet.
# SWIG_SO=<path to swig.so> loads a locally built Swig program instead.
if [[ -n "${SWIG_SO:-}" ]]; then
  SWIG_ARGS=(--upgradeable-program "$SWIG_ID" "$SWIG_SO" "$WORK/human.json")
  echo "Swig source: local build $SWIG_SO"
  shasum -a 256 "$SWIG_SO"
else
  SWIG_ARGS=(--clone-upgradeable-program "$SWIG_ID")
  echo "Swig source: official devnet program $SWIG_ID (cloned)"
fi

"$VALIDATOR" \
  --ledger "$WORK/ledger" --reset --clone-feature-set --url https://api.devnet.solana.com \
  --rpc-port 8899 --faucet-port 9900 \
  "${SWIG_ARGS[@]}" \
  --upgradeable-program "$GATE_ID" "$ROOT/adapter/target/deploy/veto_swig_gate.so" "$WORK/human.json" \
  --upgradeable-program "$TARGET_ID" "$TARGET_SO" "$WORK/human.json" \
  --upgradeable-program "$PAY_ID" "$ROOT/$PAY_DEPLOY/merchant_pay_v1.so" "$WORK/protocol.json" \
  --upgradeable-program "$ROUTER_ID" "$ROOT/$ROUTER_SO" "$WORK/human.json" \
  --upgradeable-program "$SNEAKY_ID" "$ROOT/$SNEAKY_SO" "$WORK/human.json" \
  --upgradeable-program "$RELAY_ID" "$ROOT/$RELAY_SO" "$WORK/human.json" \
  >"$VALIDATOR_LOG" 2>&1 &
VALIDATOR_PID=$!

READY=0
for _ in $(seq 1 120); do
  if "$SOLANA" cluster-version --url "$RPC_URL" >/dev/null 2>&1; then
    READY=1
    break
  fi
  if ! kill -0 "$VALIDATOR_PID" 2>/dev/null; then
    break
  fi
  sleep 0.25
done
if [[ "$READY" != "1" ]]; then
  if [[ -f "$WORK/ledger/validator.log" ]]; then
    tail -100 "$WORK/ledger/validator.log" >>"$VALIDATOR_LOG"
  fi
  echo "local validator did not become ready; see $VALIDATOR_LOG" >&2
  exit 1
fi

"$SOLANA" airdrop 10 "$HUMAN_ID" --url "$RPC_URL" >/dev/null
"$SOLANA" airdrop 10 "$AGENT_ID" --url "$RPC_URL" >/dev/null

if [[ "$RUN_MODE" == "--operator" || "$RUN_MODE" == "--operator-smoke" || "$RUN_MODE" == "--external-operator-smoke" || "$RUN_MODE" == "--external-operator-browser" ]]; then
  if [[ "$RUN_MODE" == "--external-operator-smoke" || "$RUN_MODE" == "--external-operator-browser" ]]; then
    SERVER_OPERATOR_PUBKEY="$OPERATOR_ID"
  elif [[ "$RUN_MODE" == "--operator" ]]; then
    SERVER_OPERATOR_PUBKEY="$OPERATOR_PUBKEY"
  else
    SERVER_OPERATOR_PUBKEY="$HUMAN_ID"
  fi
  env \
    OPERATOR_PUBKEY="$SERVER_OPERATOR_PUBKEY" \
    BIND_ADDR="127.0.0.1:$VETO_PORT" \
    RPC_URL="$RPC_URL" \
    HUMAN_PATH="$WORK/human.json" \
    AGENT_PATH="$WORK/agent.json" \
    PROTOCOL_PATH="$WORK/protocol.json" \
    GATE_ID="$GATE_ID" \
    PAY_ID="$PAY_ID" \
    PAY_KEYPAIR="$WORK/pay.json" \
    PAY_V1_SO="$ROOT/$PAY_DEPLOY/merchant_pay_v1.so" \
    PAY_V2_SO="$ROOT/$PAY_DEPLOY/merchant_pay_v2.so" \
    SOLANA_BIN="$SOLANA" \
    "$ROOT/adapter/target/debug/operator_server" &
  SERVER_PID=$!

  OPERATOR_URL="http://127.0.0.1:$VETO_PORT"
  OPERATOR_READY=0
  for _ in $(seq 1 120); do
    if curl --fail --silent "$OPERATOR_URL/api/status" >/dev/null 2>&1; then
      OPERATOR_READY=1
      break
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      echo "operator server exited before becoming ready; see $RUN_LOG" >&2
      exit 1
    fi
    sleep 0.25
  done
  if [[ "$OPERATOR_READY" != "1" ]]; then
    echo "operator server did not become ready" >&2
    exit 1
  fi

  if [[ "$RUN_MODE" == "--external-operator-browser" ]]; then
    VETO_URL="$OPERATOR_URL" OPERATOR_KEYPAIR="$WORK/operator.json" npm --prefix adapter/operator test
    echo "external-operator-browser-passed"
  elif [[ "$RUN_MODE" == "--external-operator-smoke" ]]; then
    post_action() {
      local step="$1"
      curl --fail --silent --show-error \
        --header "Origin: $OPERATOR_URL" \
        --header "Content-Type: application/json" \
        --request POST --data '{}' \
        "$OPERATOR_URL/api/action/$step"
    }
    prepare_approval() {
      curl --fail --silent --show-error \
        --header "Origin: $OPERATOR_URL" \
        --header "Content-Type: application/json" \
        --request POST --data '{}' \
        "$OPERATOR_URL/api/operator/approval-transaction"
    }
    submit_approval() {
      local payload="$1"
      curl --silent --show-error \
        --header "Origin: $OPERATOR_URL" \
        --header "Content-Type: application/json" \
        --request POST --data "$payload" \
        --write-out $'\n%{http_code}' \
        "$OPERATOR_URL/api/operator/submit-approval"
    }
    transaction_from_json() {
      python3 -c 'import json,sys; print(json.load(sys.stdin)["transaction"])'
    }
    transaction_payload() {
      python3 -c 'import json,sys; print(json.dumps({"transaction": sys.stdin.read().strip()}))'
    }

    INITIAL="$(curl --fail --silent "$OPERATOR_URL/api/status")"
    printf 'external-status-before=%s\n' "$INITIAL"
    grep -q '"phase":"awaitingApproval"' <<<"$INITIAL"
    grep -q '"externalOperator":true' <<<"$INITIAL"
    grep -q '"approvedSlot":null' <<<"$INITIAL"

    PREPARED="$(prepare_approval)"
    PREPARED_TX="$(transaction_from_json <<<"$PREPARED")"
    UNSIGNED_PAYLOAD="$(transaction_payload <<<"$PREPARED_TX")"
    UNSIGNED_RESULT="$(submit_approval "$UNSIGNED_PAYLOAD")"
    printf 'unsigned-approval=%s\n' "$UNSIGNED_RESULT"
    grep -q 'missing a valid authority signature' <<<"$UNSIGNED_RESULT"
    [[ "${UNSIGNED_RESULT##*$'\n'}" == "409" ]]

    if "$ROOT/adapter/target/debug/external_operator_signer" \
      "$WORK/agent.json" "$PREPARED_TX" >"$WORK/wrong-signer.out" 2>"$WORK/wrong-signer.err"; then
      echo "agent unexpectedly signed the operator transaction" >&2
      exit 1
    fi
    grep -q 'not a required signer' "$WORK/wrong-signer.err"
    echo "agent-signature-rejected"

    SIGNED_TX="$("$ROOT/adapter/target/debug/external_operator_signer" "$WORK/operator.json" "$PREPARED_TX")"
    SIGNED_PAYLOAD="$(transaction_payload <<<"$SIGNED_TX")"
    INITIALIZED_RESULT="$(submit_approval "$SIGNED_PAYLOAD")"
    printf 'external-initialize=%s\n' "$INITIALIZED_RESULT"
    [[ "${INITIALIZED_RESULT##*$'\n'}" == "200" ]]
    grep -q '"phase":"ready"' <<<"$INITIALIZED_RESULT"

    for step in execute upgrade; do
      RESPONSE="$(post_action "$step")"
      printf 'external-action-%s=%s\n' "$step" "$RESPONSE"
    done
    grep -q '"codeMatchesReviewed":false' <<<"$RESPONSE"

    UNREVIEWED_APPROVAL="$(curl --silent --show-error \
      --header "Origin: $OPERATOR_URL" \
      --header "Content-Type: application/json" \
      --request POST --data '{}' \
      --write-out $'\n%{http_code}' \
      "$OPERATOR_URL/api/operator/approval-transaction")"
    printf 'unreviewed-approval=%s\n' "$UNREVIEWED_APPROVAL"
    grep -q 'does not match the reviewed build' <<<"$UNREVIEWED_APPROVAL"
    [[ "${UNREVIEWED_APPROVAL##*$'\n'}" == "409" ]]

    RESPONSE="$(post_action probe)"
    printf 'external-action-probe=%s\n' "$RESPONSE"
    grep -q '"phase":"blocked"' <<<"$RESPONSE"
    grep -q '"veto":"490.00"' <<<"$RESPONSE"
    grep -q '"plain":"0.00"' <<<"$RESPONSE"

    RESPONSE="$(post_action fix)"
    printf 'external-action-fix=%s\n' "$RESPONSE"
    grep -q '"phase":"fixed"' <<<"$RESPONSE"
    grep -q '"codeMatchesReviewed":true' <<<"$RESPONSE"

    # Regression: code changed out-of-band while the page phase still says
    # "fixed". The console must refuse to prepare an approval for it.
    out_of_band_deploy() {
      "$SOLANA" program deploy --url "$RPC_URL" --keypair "$WORK/protocol.json" \
        --upgrade-authority "$WORK/protocol.json" --program-id "$WORK/pay.json" "$1" >/dev/null
      sleep 2
    }
    out_of_band_deploy "$ROOT/$PAY_DEPLOY/merchant_pay_v2.so"
    UNREVIEWED_FIXED="$(curl --silent --show-error \
      --header "Origin: $OPERATOR_URL" \
      --header "Content-Type: application/json" \
      --request POST --data '{}' \
      --write-out $'\n%{http_code}' \
      "$OPERATOR_URL/api/operator/approval-transaction")"
    printf 'out-of-band-upgrade-approval=%s\n' "$UNREVIEWED_FIXED"
    grep -q 'does not match the reviewed build' <<<"$UNREVIEWED_FIXED"
    [[ "${UNREVIEWED_FIXED##*$'\n'}" == "409" ]]
    out_of_band_deploy "$ROOT/$PAY_DEPLOY/merchant_pay_v1.so"
    echo "out-of-band-unreviewed-approval-refused"

    BACKEND_REAPPROVE="$(curl --silent --show-error \
      --header "Origin: $OPERATOR_URL" \
      --header "Content-Type: application/json" \
      --request POST --data '{}' \
      --write-out $'\n%{http_code}' \
      "$OPERATOR_URL/api/action/reapprove")"
    printf 'backend-reapprove=%s\n' "$BACKEND_REAPPROVE"
    grep -q 'external operator signature required' <<<"$BACKEND_REAPPROVE"
    [[ "${BACKEND_REAPPROVE##*$'\n'}" == "409" ]]

    REAPPROVAL="$(prepare_approval)"
    REAPPROVAL_TX="$(transaction_from_json <<<"$REAPPROVAL")"
    if "$ROOT/adapter/target/debug/external_operator_signer" \
      "$WORK/agent.json" "$REAPPROVAL_TX" >"$WORK/wrong-reapproval.out" 2>"$WORK/wrong-reapproval.err"; then
      echo "agent unexpectedly signed the reapproval transaction" >&2
      exit 1
    fi
    grep -q 'not a required signer' "$WORK/wrong-reapproval.err"
    echo "agent-reapproval-rejected"

    SIGNED_REAPPROVAL="$("$ROOT/adapter/target/debug/external_operator_signer" "$WORK/operator.json" "$REAPPROVAL_TX")"
    REAPPROVAL_PAYLOAD="$(transaction_payload <<<"$SIGNED_REAPPROVAL")"
    REAPPROVED_RESULT="$(submit_approval "$REAPPROVAL_PAYLOAD")"
    printf 'external-reapprove=%s\n' "$REAPPROVED_RESULT"
    [[ "${REAPPROVED_RESULT##*$'\n'}" == "200" ]]
    grep -q '"phase":"reapproved"' <<<"$REAPPROVED_RESULT"

    RESPONSE="$(post_action resume)"
    printf 'external-action-resume=%s\n' "$RESPONSE"
    grep -q '"phase":"resumed"' <<<"$RESPONSE"
    grep -q '"veto":"480.00"' <<<"$RESPONSE"
    echo "external-operator-smoke-passed"
  elif [[ "$RUN_MODE" == "--operator-smoke" ]]; then
    PAGE="$(curl --fail --silent "$OPERATOR_URL/")"
    grep -q "An upgrade should stop the agent" <<<"$PAGE"
    curl --fail --silent "$OPERATOR_URL/tokens.css" | grep -q -- "--color-accent"
    INITIAL="$(curl --fail --silent "$OPERATOR_URL/api/status")"
    printf 'status-before=%s\n' "$INITIAL"
    grep -q '"phase":"ready"' <<<"$INITIAL"
    for step in execute upgrade probe fix reapprove resume; do
      RESPONSE="$(curl --fail --silent --show-error \
        --header "Origin: $OPERATOR_URL" \
        --header "Content-Type: application/json" \
        --request POST --data '{}' \
        "$OPERATOR_URL/api/action/$step")"
      printf 'action-%s=%s\n' "$step" "$RESPONSE"
      case "$step" in
        execute) EXPECTED='"phase":"executed"' ;;
        upgrade) EXPECTED='"phase":"upgraded"' ;;
        probe) EXPECTED='"phase":"blocked"' ;;
        fix) EXPECTED='"phase":"fixed"' ;;
        reapprove) EXPECTED='"phase":"reapproved"' ;;
        resume) EXPECTED='"phase":"resumed"' ;;
      esac
      grep -q "$EXPECTED" <<<"$RESPONSE"
    done
    grep -q '"veto":"480.00"' <<<"$RESPONSE"
    # A client that trickles one header byte every 3 s must be cut off at the
    # server's 10 s request deadline rather than holding a thread open.
    SLOW_SECONDS="$(python3 - "$VETO_PORT" <<'PY'
import socket, sys, time
sock = socket.create_connection(("127.0.0.1", int(sys.argv[1])))
sock.sendall(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n")
started = time.time()
try:
    for _ in range(20):
        time.sleep(3)
        sock.sendall(b"X")
except OSError:
    pass
print(round(time.time() - started))
PY
)"
    printf 'slow-client-cut-off-after=%ss\n' "$SLOW_SECONDS"
    (( SLOW_SECONDS <= 20 ))
    echo "operator-smoke-passed"
  else
    echo "Open $OPERATOR_URL in a browser. Press Ctrl-C to stop and remove temporary demo state."
    wait "$SERVER_PID"
  fi
elif [[ "$RUN_MODE" == "--route" ]]; then
  env \
  RPC_URL="$RPC_URL" \
  HUMAN_PATH="$WORK/human.json" \
  AGENT_PATH="$WORK/agent.json" \
  PROTOCOL_PATH="$WORK/protocol.json" \
  GATE_ID="$GATE_ID" \
  ROUTER_ID="$ROUTER_ID" \
  SNEAKY_ID="$SNEAKY_ID" \
  PAY_ID="$PAY_ID" \
  PAY_KEYPAIR="$WORK/pay.json" \
  PAY_V1_SO="$ROOT/$PAY_DEPLOY/merchant_pay_v1.so" \
  PAY_V2_SO="$ROOT/$PAY_DEPLOY/merchant_pay_v2.so" \
  SOLANA_BIN="$SOLANA" \
  "$ROOT/adapter/target/debug/route_flow"

  grep -q '^route-flow-passed$' "$RUN_LOG"
  echo "Veto route demo passed. Evidence: $RUN_LOG"
elif [[ "$RUN_MODE" == "--payment" ]]; then
  env \
  RPC_URL="$RPC_URL" \
  HUMAN_PATH="$WORK/human.json" \
  AGENT_PATH="$WORK/agent.json" \
  PROTOCOL_PATH="$WORK/protocol.json" \
  GATE_ID="$GATE_ID" \
  PAY_ID="$PAY_ID" \
  PAY_KEYPAIR="$WORK/pay.json" \
  PAY_V1_SO="$ROOT/$PAY_DEPLOY/merchant_pay_v1.so" \
  PAY_V2_SO="$ROOT/$PAY_DEPLOY/merchant_pay_v2.so" \
  SOLANA_BIN="$SOLANA" \
  "$ROOT/adapter/target/debug/payment_flow"

  grep -q '^payment-flow-passed$' "$RUN_LOG"
  echo "Veto payment demo passed. Evidence: $RUN_LOG"
else
  env \
  RPC_URL="$RPC_URL" \
  HUMAN_PATH="$WORK/human.json" \
  AGENT_PATH="$WORK/agent.json" \
  GATE_ID="$GATE_ID" \
  TARGET_ID="$TARGET_ID" \
  RELAY_ID="$RELAY_ID" \
  TARGET_KEYPAIR="$WORK/target.json" \
  TARGET_SO="$TARGET_SO" \
  SOLANA_BIN="$SOLANA" \
  "$ROOT/adapter/target/debug/local_swig_flow"

  grep -q '^complete-flow-passed$' "$RUN_LOG"
  echo "Veto demo passed. Evidence: $RUN_LOG"
fi
