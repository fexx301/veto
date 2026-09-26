#!/usr/bin/env bash
# Runs on a fresh Ubuntu 24.04 host as the default `ubuntu` user.
# Expects the source tree in ~/veto (see push.sh) and devnet keys in
# ~/veto-keys. Installs build tools, Rust, the Solana CLI and Caddy; builds the
# operator server; installs the systemd service and HTTPS reverse proxy.
#
#   DOMAIN=<host name> [REDIRECT_FROM=<old host name>] ./provision.sh
set -euo pipefail
: "${DOMAIN:?set DOMAIN, e.g. 3-91-20-5.sslip.io}"
SRC="$HOME/veto"
KEYS="$HOME/veto-keys"
AGAVE_VERSION="${AGAVE_VERSION:-v4.2.1}"

# Values below are written into /etc/veto.env and the Caddyfile; refuse
# anything that could add a line or directive.
host_name() { [[ -z "$1" || "$1" =~ ^[A-Za-z0-9.-]+$ ]] || { echo "invalid host name: $1" >&2; exit 1; }; }
host_name "$DOMAIN"
host_name "${REDIRECT_FROM:-}"

# 4 GiB swap so the Rust build fits a 2 GiB instance.
if ! swapon --show | grep -q /swapfile; then
  sudo fallocate -l 4G /swapfile
  sudo chmod 600 /swapfile
  sudo mkswap /swapfile >/dev/null
  sudo swapon /swapfile
  echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab >/dev/null
fi

sudo apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  build-essential pkg-config libssl-dev libudev-dev clang curl ca-certificates \
  debian-keyring debian-archive-keyring apt-transport-https gnupg

if ! command -v caddy >/dev/null; then
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    | sudo tee /etc/apt/sources.list.d/caddy-stable.list >/dev/null
  sudo apt-get update -qq
  sudo apt-get install -y -qq caddy
fi

if ! command -v cargo >/dev/null && [[ ! -x "$HOME/.cargo/bin/cargo" ]]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

SOLANA_BIN="$HOME/.local/share/solana/install/active_release/bin/solana"
if [[ ! -x "$SOLANA_BIN" ]]; then
  sh -c "$(curl -sSfL "https://release.anza.xyz/$AGAVE_VERSION/install")"
fi
"$SOLANA_BIN" --version

cd "$SRC"
cargo build --release --locked --manifest-path adapter/Cargo.toml --features client --bin operator_server

sudo install -d -m 755 /opt/veto /opt/veto/programs
sudo install -m 755 adapter/target/release/operator_server /opt/veto/operator_server
sudo install -m 644 adapter/fixtures/merchant-pay/target/deploy/merchant_pay_v1.so /opt/veto/programs/
sudo install -m 644 adapter/fixtures/merchant-pay/target/deploy/merchant_pay_v2.so /opt/veto/programs/
id veto >/dev/null 2>&1 || sudo useradd --system --create-home --home-dir /var/lib/veto veto
sudo install -d -o veto -g veto -m 700 /var/lib/veto/keys
for key in deployer agent protocol pay; do
  sudo install -o veto -g veto -m 600 "$KEYS/$key.json" /var/lib/veto/keys/
done
sudo install -d -m 755 /opt/veto/solana
sudo cp -r "$(dirname "$(readlink -f "$SOLANA_BIN")")"/. /opt/veto/solana/

# shellcheck disable=SC1091
source "$KEYS/env"
for id in "$GATE_ID" "$PAY_ID"; do
  [[ "$id" =~ ^[1-9A-HJ-NP-Za-km-z]{32,44}$ ]] || { echo "invalid program id: $id" >&2; exit 1; }
done
RPC_URL="${RPC_URL:-https://api.devnet.solana.com}"
[[ "$RPC_URL" =~ ^https://[^[:space:]]+$ ]] || { echo "RPC_URL must be a single https:// URL" >&2; exit 1; }
[[ "${MAX_RUNS_PER_HOUR:-20}" =~ ^[0-9]+$ ]] || { echo "invalid MAX_RUNS_PER_HOUR" >&2; exit 1; }
sudo tee /etc/veto.env >/dev/null <<ENV
HOSTED=1
PUBLIC_ORIGIN=https://$DOMAIN
BIND_ADDR=127.0.0.1:4173
RPC_URL=$RPC_URL
MAX_RUNS_PER_HOUR=${MAX_RUNS_PER_HOUR:-20}
HUMAN_PATH=/var/lib/veto/keys/deployer.json
AGENT_PATH=/var/lib/veto/keys/agent.json
PROTOCOL_PATH=/var/lib/veto/keys/protocol.json
PAY_KEYPAIR=/var/lib/veto/keys/pay.json
GATE_ID=$GATE_ID
PAY_ID=$PAY_ID
PAY_V1_SO=/opt/veto/programs/merchant_pay_v1.so
PAY_V2_SO=/opt/veto/programs/merchant_pay_v2.so
SOLANA_BIN=/opt/veto/solana/solana
ENV
sudo chmod 640 /etc/veto.env

sudo install -m 644 "$SRC/adapter/deploy/aws/veto.service" /etc/systemd/system/veto.service
sudo tee /etc/caddy/Caddyfile >/dev/null <<CADDY
$DOMAIN {
  encode gzip
  header Strict-Transport-Security "max-age=31536000"
  # Caddy replaces any X-Forwarded-For a client sends; the server uses it
  # for its per-client run limit.
  reverse_proxy 127.0.0.1:4173
}
CADDY
# REDIRECT_FROM: optional older host name that should redirect to DOMAIN.
if [[ -n "${REDIRECT_FROM:-}" ]]; then
  sudo tee -a /etc/caddy/Caddyfile >/dev/null <<CADDY

$REDIRECT_FROM {
  redir https://$DOMAIN{uri} permanent
}
CADDY
fi
sudo systemctl daemon-reload
sudo systemctl enable --now veto
sudo systemctl restart veto
sudo systemctl reload caddy || sudo systemctl restart caddy
sleep 3
systemctl --no-pager --lines=5 status veto
echo "provisioned https://$DOMAIN"
