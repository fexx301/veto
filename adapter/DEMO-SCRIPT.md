# Veto operator walkthrough

## Run

From the checkout root, with a fresh test wallet (for example Phantom) that
holds no real funds:

```sh
OPERATOR_PUBKEY=<fresh-test-wallet-address> \
  ./adapter/scripts/run-local-demo.sh --operator
```

Open `http://127.0.0.1:4173` in the browser that has the wallet extension.
Choose **Connect wallet & approve**, then follow the highlighted action:
**Pay 10 from both**, **Upgrade merchant-pay**, **Request 10 from both**,
**Redeploy reviewed build**, **Review & sign deployment**, **Pay 10 through
Veto**. The server prepares and fee-pays each transaction; your wallet signs
the two approvals. Stop with Ctrl-C.

## Narration

AI agents increasingly hold delegated wallets. A typical guardrail says "the
agent may call this program, up to this budget." But on Solana an upgradeable
program keeps its address when its code changes, so that allowlist keeps
trusting whatever code the program's upgrade key deploys next.

Two Swig wallets give the same agent the same 500 test-token budget for the
same merchant-pay program. One uses an ordinary program allowlist. The other
routes the agent through Veto, which records the exact deployment the operator
approved. The operator's wallet is the root of the protected wallet and the
only key that can approve a deployment.

Under the reviewed build, the agent pays 10 from each wallet. Then the
protocol's upgrade key ships v2 under the same address; v2 charges the whole
balance. The agent asks for 10 again. The allowlisted wallet is charged 490.
Veto rejects the call on-chain and the protected wallet keeps its 490.

The protocol ships a fix: the reviewed build again. The console shows the
deployed code matches the build the operator reviewed, but Veto keeps
payments paused until the operator signs. After approval, the agent pays 10
through Veto.

## Honest limits

- v2 is a deliberately malicious fixture written for this demo, not a replay
  of a real incident. Swig's spending limit is what capped the allowlisted
  wallet's loss at its budget.
- Veto enforces the deployment slot. The code-hash comparison is a console
  aid for review, not a reproducible-build attestation or a safety verdict.
- Veto checks every program the call can reach (merchant-pay and SPL Token
  here) against the operator's approvals; it does not judge what approved
  code does.
- The comparison wallet's root is the server key because it is only a
  control; the protected wallet's root is the operator.
- User demand, adoption, and willingness to pay are unvalidated.
