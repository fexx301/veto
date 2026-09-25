#!/usr/bin/env bash
# Copies the committed source over any previous copy (keeping the build
# cache in ~/veto/adapter/target), the prebuilt merchant-pay builds, and the
# devnet keys to the host, then runs provision.sh there.
#
#   HOST=ubuntu@<ip> SSH_KEY=~/.ssh/veto-demo.pem DOMAIN=<name> ./adapter/deploy/aws/push.sh
set -euo pipefail
: "${HOST:?set HOST=ubuntu@<ip>}" "${DOMAIN:?set DOMAIN}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
KEYS="${VETO_DEVNET_DIR:-$HOME/.config/veto-devnet}"
SSH=(ssh -o StrictHostKeyChecking=accept-new ${SSH_KEY:+-i "$SSH_KEY"})
PAY="adapter/fixtures/merchant-pay/target/deploy"
cd "$ROOT"
"${SSH[@]}" "$HOST" 'mkdir -p ~/veto ~/veto-keys && chmod 700 ~/veto-keys'
git archive --format=tar HEAD | "${SSH[@]}" "$HOST" 'tar -x -C ~/veto'
tar -c "$PAY/merchant_pay_v1.so" "$PAY/merchant_pay_v2.so" | "${SSH[@]}" "$HOST" 'tar -x -C ~/veto'
(cd "$KEYS" && tar -c deployer.json agent.json protocol.json pay.json env) \
  | "${SSH[@]}" "$HOST" 'tar -x -C ~/veto-keys && chmod 600 ~/veto-keys/*'
"${SSH[@]}" "$HOST" "cd ~/veto && DOMAIN='$DOMAIN' REDIRECT_FROM='${REDIRECT_FROM:-}' bash adapter/deploy/aws/provision.sh"
