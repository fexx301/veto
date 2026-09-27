# Veto authority map

Prepared 2026-09-24 from source inspection of this checkout (Swig commit
`3e0411f0c2980b26296903eaa4aa0c2d07869316` plus the local Veto adapter) and
read-only public RPC queries. Findings marked **source inference** have not
been executed as an attack; findings marked **executed** have log evidence.

## Who controls what

Updated 2026-09-27. "Operator" is the wallet that approves deployments: the
visitor's wallet in hosted and `--operator` runs, or the server's test key in
the scripted local runs and hosted demo-operator runs.

| Authority | Held by | Can do | Source |
|---|---|---|---|
| Swig root role (Ed25519, `All`) | Operator (server `human`/setup key in scripted and demo-operator runs) | Sign any wallet action, add/remove/replace roles, remove Veto's role | `demo/payment.rs` `swig_create` |
| Swig `ProgramExec` role (Veto) | Nobody (program-authenticated) | Call the approved target through the wallet when the preceding instruction is Veto `[2, policy]` for this config/wallet | `state/src/authority/programexec/mod.rs` |
| Agent | Server `agent` key, stored in the policy | Submit the Veto proof + `SignV2`; the gate requires this key as signer | `adapter/src/lib.rs` `authorize_next_swig` |
| Veto policy authority | Operator | Initialize the policy; reapprove a deployment; approve route programs | `adapter/src/lib.rs` `initialize`, `reapprove`, `approve_route_program` |
| Policy account creation | Operator-signed setup transaction (policy key co-signs; server pays) | Create, initialize and bind the policy atomically | `demo/payment.rs` `setup_ixs` |
| Merchant-pay upgrade authority | Server `protocol` key (stands in for the protocol team) | Upgrade the payment target | `run-local-demo.sh`, `deploy-devnet.sh` |
| Counter, router, relay fixtures' upgrade authority (local) | Server `human` key | Upgrade those fixtures | `run-local-demo.sh` |
| Veto gate upgrade authority | Local: server `human` key. Devnet: a `gate-authority` key never copied to the hosted server; not finalized | Replace the gate code | `run-local-demo.sh`, `deploy-devnet.sh` |
| Swig program upgrade authority | Swig team (devnet program cloned by default); server `human` key only with `SWIG_SO` | Replace the wallet program | `run-local-demo.sh` |
| Fee payer / transaction preparer | Server setup key | Pays fees; builds the approval transaction the operator signs | `prepare_operator_approval` |

## What Veto does not constrain

Veto constrains one delegated role: the agent's. It does not constrain the
wallet's root. A root with `All` can sign a payment directly, add an unguarded
role for the agent, or remove or replace the Veto role; Swig never consults
Veto for those. Veto's guarantee therefore holds only while the root is a key
the operator alone controls. In hosted runs with your own wallet that is your
wallet; in scripted local runs and hosted demo-operator runs the server holds
the root, so those runs demonstrate the mechanism, not operator ownership.

Veto also trusts the programs it cannot pin itself:

- **The Swig program.** Swig, not Veto, enforces that a `ProgramExec` role
  needs Veto's preceding instruction. Pinning Swig's deployment slot in Veto
  would not help: an upgraded Swig that stopped enforcing `ProgramExec` would
  simply stop requiring Veto's instruction. Swig's upgrade authority is a
  trust dependency, as for any Swig wallet.
- **The Veto gate itself.** Whoever can upgrade the gate can replace its
  checks while every Swig role keeps pointing at it. On devnet that key is
  held off the hosted server; finalizing the gate would remove it entirely,
  and is deferred so fixes can still be deployed.

## Why a deployment slot identifies code

Veto stores a deployment slot, not a code hash. That is enough because, in
the upgradeable loader (`solana-bpf-loader-program` 3.1.12, `process_instruction`):

- program bytes reach ProgramData only through `Upgrade` (and `DeployWithMaxDataLen`
  at creation); `Write` only writes buffers;
- `Upgrade` records the current slot and refuses a second upgrade in the same
  slot ("Program was deployed in this block already"), so two different codes
  cannot share one slot;
- a closed program cannot be redeployed at its address, and `Migrate` moves a
  program to loader v4, which the gate refuses (`UNSUPPORTED_LOADER`,
  `TARGET_MISMATCH`).

Hashing each program on-chain on every delegated call would cost tens of
thousands of compute units per program for no additional guarantee. What the
operator reviews is still off-chain: the console compares the deployed code
hash with the reviewed build before it prepares an approval.

## Review decisions

- **Pinning Swig's own slot (not done).** A replaced Swig program would not
  need to call the gate at all, so pinning it inside Veto cannot stop that;
  Swig's upgrade authority is a stated trust dependency (above).
- **Open hosted demo (by design).** Anyone may start a run so judges and
  users can try it without an account. Runs are per-visitor (cookie),
  limited per client and overall, time-capped, and the demo operator's
  server-held root is labelled on the page. All status fields are public
  devnet addresses and balances.
- **Role id not bound in the policy.** Another role in the same wallet (for
  example a session role) bypasses Veto, but only the wallet root can create
  one; see "What Veto does not constrain".
- **Reserved programs.** The wallet program and the gate can never be a
  delegated call's target or route (`RESERVED_PROGRAM`, 23).

## Findings

**F1 — The delegated route has no agent identity (critical before any value-moving demo; source inference).**
Swig's `ProgramExec` authentication checks only the preceding instruction's
program, data prefix, and config/wallet accounts. It checks no signer. Veto's
`authorize_next_swig` checks no signer either. Any keypair can therefore submit
the Veto proof plus `SignV2` and invoke the approved target through the wallet,
with arbitrary accounts and data, within the role's other limits. For the
counter fixture that is harmless; for a payment target it would let anyone
trigger payments.
*Fix:* store the agent public key in the policy (set by the operator) and
require that key as a signer in `authorize_next_swig`. Test: a non-agent payer
is rejected with no state change.

**F2 — The server holds the Swig root, so the demo is not operator-owned (high; source inference).**
Swig's root has `All` permission. It can remove Veto's role, add an unguarded
role for the agent, or move funds directly. This is correct Swig behaviour, not
a Veto defect, but it means the demo cannot claim that only the operator
controls the wallet.
*Fix:* create the Swig with the operator's key as root. Swig `Create` does not
require the root's signature (`interface/src/lib.rs`, `CreateInstruction::new`
accounts: config, payer, wallet, system), so the server can still pay.
`AddAuthority` requires the root's signature, so the operator signs role setup.

**F3 — Policy initialization can be front-run (high; source inference).**
The server creates the policy account and binds the Swig role prefix
`[2, policy]` in transactions *before* the operator initializes the policy in a
separate transaction. `initialize` accepts any signer as authority on an
uninitialized account. Anyone watching the chain could initialize it first,
naming themselves as the approval authority.
*Fix:* create the policy account, initialize it, and add the Swig role in one
operator-signed transaction (atomic), or derive the policy as a PDA from the
Swig config and operator key.

**F4 — Program upgrade authorities are trust dependencies (medium; verified by RPC).**
Whoever can upgrade Veto can replace the gate. For devnet, deploy Veto and then
make it immutable (`solana program set-upgrade-authority --final`) or disclose
the authority. The official Swig program `swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB`
is itself upgradeable on devnet and mainnet (ProgramData
`Bb6gN8CtkMXf7cfXKnWysmdBg5B8EZfP5kus5TsyH5Es`, authority
`8MjgP7L5kHfv52s15ekpewSJiCkismhwYLFJmuFjh6CP`; last deployed slot 495820019 on
devnet and 445747246 on mainnet, read 2026-09-24). Veto does not pin the
wallet program itself; an optional extension could pin Swig's slot the same
way.

**F5 — Roles are collapsed onto one key in the demo (low).**
The same server key is Swig root, fee payer, policy creator, and target
upgrader. The devnet demo should use a separate "protocol team" key for the
target upgrade so the story matches the trust model.

## Constraints found for the payment demo

- A Swig role with only `Program` permission cannot reduce a Swig-owned token
  account balance; `SignV2` requires a `TokenLimit`-family permission for the
  mint (`program/src/actions/sign_v2.rs`, `PermissionDeniedMissingPermission`).
- Swig's destination-limit parsing only reads direct SPL Token instructions in
  the compact payload (`process_token_destinations`), so when payment goes
  through a target program only an amount limit applies.
- Correction (2026-09-25): SPL Token runs on the upgradeable loader on
  mainnet and devnet, with no upgrade authority (finalized; its devnet and
  mainnet deployment slots differ because runtime feature migrations rewrite
  core programs). The demo target is still a merchant-pay program so that a
  real code-changing upgrade can be shown; SPL Token is approved as a
  downstream program at setup.

## Status

F1–F3 fixed and re-tested on 2026-09-24; see the last section of
[`verification-logs/README.md`](../verification-logs/README.md). F4 and F5 are
handled at devnet deployment.

## Go / no-go

**Go.** None of the findings needs a Swig change. F1 and F3 are Veto adapter
and setup changes; F2 is a setup change; F4 and F5 are deployment choices. The
promise "only the operator can approve which deployed code the agent may call"
holds only after F1–F3 are fixed and re-tested on a fresh validator.
