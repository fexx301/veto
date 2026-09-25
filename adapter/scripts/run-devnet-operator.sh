#!/usr/bin/env bash
# Serves the operator console on http://127.0.0.1:4173 against Solana devnet,
# using the programs deployed by deploy-devnet.sh and the official Swig program.
#
#   OPERATOR_PUBKEY=<test-wallet-address> ./adapter/scripts/run-devnet-operator.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SOLANA_TOOLS="${SOLANA_TOOLS:-$HOME/.local/share/solana/install/active_release/bin}"
DIR="${VETO_DEVNET_DIR:-$HOME/.config/veto-devnet}"
PAY_DEPLOY="$ROOT/adapter/fixtures/merchant-pay/target/deploy"
: "${OPERATOR_PUBKEY:?set OPERATOR_PUBKEY to the operator test wallet address}"
[[ -f "$DIR/env" ]] || { echo "run adapter/scripts/deploy-devnet.sh first" >&2; exit 1; }
# shellcheck disable=SC1091
source "$DIR/env"

mkdir -p "$ROOT/verification-logs"
LOG="$ROOT/verification-logs/devnet-operator-$(date -u +%Y%m%dT%H%M%SZ).log"
echo "Logging to $LOG"
env \
  OPERATOR_PUBKEY="$OPERATOR_PUBKEY" \
  RPC_URL="https://api.devnet.solana.com" \
  HUMAN_PATH="$DIR/deployer.json" \
  AGENT_PATH="$DIR/agent.json" \
  PROTOCOL_PATH="$DIR/protocol.json" \
  GATE_ID="$GATE_ID" \
  PAY_ID="$PAY_ID" \
  PAY_KEYPAIR="$DIR/pay.json" \
  PAY_V1_SO="$PAY_DEPLOY/merchant_pay_v1.so" \
  PAY_V2_SO="$PAY_DEPLOY/merchant_pay_v2.so" \
  SOLANA_BIN="$SOLANA_TOOLS/solana" \
  "$ROOT/adapter/target/debug/operator_server" 2>&1 | tee "$LOG"
