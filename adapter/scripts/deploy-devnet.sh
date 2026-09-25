#!/usr/bin/env bash
# Deploys the Veto gate and the merchant-pay v1 fixture to Solana devnet.
# Uses the official Swig program already on devnet. Keys live outside the
# repository in $VETO_DEVNET_DIR (default ~/.config/veto-devnet).
#
#   ./adapter/scripts/deploy-devnet.sh              # build and deploy
#   ./adapter/scripts/deploy-devnet.sh --finalize   # make Veto immutable
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SOLANA_TOOLS="${SOLANA_TOOLS:-$HOME/.local/share/solana/install/active_release/bin}"
SOLANA="$SOLANA_TOOLS/solana"
KEYGEN="$SOLANA_TOOLS/solana-keygen"
BUILD_SBF="$SOLANA_TOOLS/cargo-build-sbf"
DIR="${VETO_DEVNET_DIR:-$HOME/.config/veto-devnet}"
URL="https://api.devnet.solana.com"
SWIG_ID="swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB"
PAY_DEPLOY="$ROOT/adapter/fixtures/merchant-pay/target/deploy"
GATE_SO="$ROOT/adapter/target/deploy/veto_swig_gate.so"
cd "$ROOT"

mkdir -p "$DIR" && chmod 700 "$DIR"
for key in deployer gate pay protocol agent; do
  [[ -f "$DIR/$key.json" ]] || "$KEYGEN" new --no-bip39-passphrase --silent --outfile "$DIR/$key.json"
  chmod 600 "$DIR/$key.json"
done
DEPLOYER="$("$KEYGEN" pubkey "$DIR/deployer.json")"
GATE_ID="$("$KEYGEN" pubkey "$DIR/gate.json")"
PAY_ID="$("$KEYGEN" pubkey "$DIR/pay.json")"
PROTOCOL="$("$KEYGEN" pubkey "$DIR/protocol.json")"
AGENT="$("$KEYGEN" pubkey "$DIR/agent.json")"
sol() { "$SOLANA" --url "$URL" --keypair "$DIR/deployer.json" "$@"; }

if [[ "${1:-}" == "--finalize" ]]; then
  sol program set-upgrade-authority "$GATE_ID" --final
  sol program show "$GATE_ID"
  exit 0
fi

# The Veto flow needs Swig's ProgramExec role; confirm the devnet program exists.
sol program show "$SWIG_ID" >/dev/null

BALANCE="$(sol balance --lamports "$DEPLOYER" | awk '{print $1}')"
if (( BALANCE < 2000000000 )); then
  echo "deployer $DEPLOYER has $((BALANCE / 1000000)) milli-SOL; fund at least 2 devnet SOL (5 recommended) at https://faucet.solana.com" >&2
  exit 1
fi

"$BUILD_SBF" --manifest-path adapter/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
"$BUILD_SBF" --manifest-path adapter/fixtures/merchant-pay/Cargo.toml --tools-version v1.54 --arch v3 -- --locked
cp "$PAY_DEPLOY/veto_merchant_pay.so" "$PAY_DEPLOY/merchant_pay_v1.so"
"$BUILD_SBF" --manifest-path adapter/fixtures/merchant-pay/Cargo.toml --tools-version v1.54 --arch v3 --features drain -- --locked
cp "$PAY_DEPLOY/veto_merchant_pay.so" "$PAY_DEPLOY/merchant_pay_v2.so"
cargo build --locked --manifest-path adapter/Cargo.toml --features client --bin operator_server
shasum -a 256 "$GATE_SO" "$PAY_DEPLOY/merchant_pay_v1.so" "$PAY_DEPLOY/merchant_pay_v2.so"

sol program deploy --program-id "$DIR/gate.json" "$GATE_SO"
lamports() { sol balance --lamports "$1" | awk '{print $1}'; }
(( $(lamports "$PROTOCOL") >= 500000000 )) || sol transfer --allow-unfunded-recipient "$PROTOCOL" 1 >/dev/null
(( $(lamports "$AGENT") >= 100000000 )) || sol transfer --allow-unfunded-recipient "$AGENT" 0.2 >/dev/null
# Deploy merchant-pay only if it is missing or not the reviewed v1 build.
PAY_DEPLOYED="$(sol program dump "$PAY_ID" "$DIR/pay-deployed.so" >/dev/null 2>&1 && shasum -a 256 "$DIR/pay-deployed.so" | cut -d' ' -f1 || true)"
PAY_V1_HASH="$(shasum -a 256 "$PAY_DEPLOY/merchant_pay_v1.so" | cut -d' ' -f1)"
rm -f "$DIR/pay-deployed.so"
if [[ "$PAY_DEPLOYED" != "$PAY_V1_HASH" ]]; then
  sol program deploy --program-id "$DIR/pay.json" --upgrade-authority "$DIR/protocol.json" "$PAY_DEPLOY/merchant_pay_v1.so"
fi

cat >"$DIR/env" <<ENV
GATE_ID=$GATE_ID
PAY_ID=$PAY_ID
DEPLOYER=$DEPLOYER
PROTOCOL=$PROTOCOL
AGENT=$AGENT
ENV
echo "devnet-deployed gate=$GATE_ID merchant-pay=$PAY_ID protocol=$PROTOCOL agent=$AGENT"
echo "Explorer: https://explorer.solana.com/address/$GATE_ID?cluster=devnet"
