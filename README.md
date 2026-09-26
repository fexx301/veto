<p align="center"><img src="adapter/operator/brand/png/veto-lockup-1200.png" alt="Veto" width="360"></p>

# Veto

**Approve the exact code your agent's wallet may call on Solana.**

An AI agent's wallet usually trusts programs by address: "this agent may call
Jupiter, up to 500 USDC." But an upgradeable Solana program keeps its address
when its code is replaced, so that permission silently extends to whatever code
is deployed next. Veto pins the deployment the operator approved. After any
upgrade, of the program the agent calls or of any program that call can reach,
the agent's call fails on-chain until the operator reviews and approves the new
deployment.

**Live demo (Solana devnet):** https://veto-demo.duckdns.org. Connect a
devnet test wallet, or use the demo operator.

## Why it matters

Measured on mainnet on 2026-09-24 across the 117 programs Jupiter routes swaps
through plus major first-hop programs ([method, data and caveats](verification-logs/2026-09-24-upgrade-study/);
counts are lower bounds):

| | |
|---|---|
| Programs that can still be upgraded | **115 of 117** |
| Upgraded in the last 180 days | **at least 67** (712 upgrades counted) |
| Upgraded weekly or faster | **27** |
| Upgrade authority is a single key | **35** (19 of them actively upgraded) |
| Jupiter Aggregator v6 upgrades in 180 days | **23** |

Even SPL Token now runs on the upgradeable loader (finalized). Upgrades are
routine and almost all are benign. An allowlist cannot tell the difference, and
an agent never stops to ask.

## What the demo shows

Two Swig wallets give the same agent the same 500-test-token budget for the
same merchant program. One uses an ordinary program allowlist; the other routes
through Veto. The program's upgrade key then ships a v2 that charges the whole
balance.

| | Plain allowlist | Veto |
|---|---|---|
| Pay 10 under the reviewed build | 490 left | 490 left |
| Upgrade key ships v2 (same address) | | |
| Agent asks to pay 10 | **charged 490** | **blocked on-chain**, 490 kept |
| Reviewed build redeployed | | still paused until the operator approves |
| Operator approves, agent pays 10 | | 480 left |

Every step is a real devnet transaction; see
[`verification-logs/DEVNET.md`](verification-logs/DEVNET.md) for explorer
links. The console also compares the deployed code hash with the reviewed build
and refuses to prepare an approval for code that does not match.

## How it works

Swig lets a wallet role authenticate through a preceding instruction
(`ProgramExec`). Veto is that instruction. Immediately before the agent's Swig
`SignV2`, Veto checks, on-chain and in the same transaction, that:

- the agent key, Swig config and wallet match the operator's policy;
- the target program, and **every upgradeable program the delegated call can
  reach**, is approved at its current ProgramData deployment slot;
- the proof is consumed by exactly that one `SignV2`.

Route-wide checking rests on a runtime rule, tested here on Agave 4.2.1: a
program can only invoke programs among its own instruction accounts, so every
reachable program is visible to Veto. A freshly deployed and finalized program
is not trusted implicitly. Only the operator's wallet, which is also the Swig
wallet's root, can approve a deployment. An off-chain policy check could race
an upgrade; Veto's check executes inside the transaction it protects.

The trust boundary is documented in [`adapter/AUTHORITY-MAP.md`](adapter/AUTHORITY-MAP.md).
The attack regression covers policy, agent and wallet substitution, proof
replay, front-running, nested CPI, cross-wallet reuse, and unapproved or
changed downstream programs.

## Run it

Requires Rust 1.91.0 (pinned), Solana CLI 4.2.1 and `cargo-build-sbf` 4.1.0
(platform tools v1.54). From the repository root:

```sh
./adapter/scripts/run-local-demo.sh            # attack regression
./adapter/scripts/run-local-demo.sh --payment  # two-wallet payment story
./adapter/scripts/run-local-demo.sh --route    # downstream-upgrade story
# browser console with your own test wallet as operator:
(cd adapter/operator && npm ci && npm run build)
OPERATOR_PUBKEY=<test-wallet-address> ./adapter/scripts/run-local-demo.sh --operator
```

Each run starts a fresh local validator with disposable keys, clones the
official Swig program from devnet, and removes everything on exit. Devnet and
AWS deployment scripts are in `adapter/scripts/` and `adapter/deploy/aws/`.

## Repository

| Path | Contents |
|---|---|
| `adapter/src/lib.rs` | The on-chain Veto gate |
| `adapter/src/bin/` | Validator flows, operator console server, signer |
| `adapter/fixtures/` | Test programs: merchant-pay (v1 and malicious v2), payment-router, counter, relay, sneaky-cpi |
| `adapter/operator/` | Console page, wallet bridge, browser check, brand |
| `research/` | Mainnet upgrade-frequency study |
| `verification-logs/` | Run logs, devnet records, study data |

## Limits

- Veto enforces the deployment, not what the code does. Deciding whether a new
  deployment is safe is the operator's review; the console's reviewed-build hash
  comparison is an aid, not an attestation.
- Tested routes are two programs deep plus SPL Token. A policy holds up to eight
  downstream programs, approvals can be added or re-pinned but not yet revoked,
  and legacy-loader programs are refused. Jupiter-scale routes are untested.
- One Veto-protected `SignV2` per transaction; Swig only.
- Devnet only. The gate is not audited, and its upgrade authority is not yet
  finalized.
- No public Solana incident is known in which a malicious upgrade drained users;
  Veto is a preventive control for a documented structural risk.

## Licence

AGPL-3.0. Built on [Swig](https://github.com/anagrambuild/swig-wallet) (AGPL-3.0,
Anagram); see [`adapter/DISCLOSURE.md`](adapter/DISCLOSURE.md).
