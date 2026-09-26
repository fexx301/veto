# Veto adapter: gate, fixtures, flows and console

Veto pauses a delegated Swig action when any program the action can reach no
longer matches the deployment the operator approved. Only the recorded operator
authority can approve a new deployment. The attack regression uses a valueless
counter so execution or non-execution is directly observable; the payment and
route demos use a valueless test token.

## Reproduce

Prerequisites are the pinned Rust toolchain in the repository (Rust 1.91.0) and
the verified Solana tools: Agave/Solana CLI 4.2.1, `cargo-build-sbf` 4.1.0,
platform tools v1.54 and SBF Rust 1.89.0. By default the script looks in
`$HOME/.local/share/solana/install/active_release/bin`; set `SOLANA_TOOLS` to a
different installation directory if needed.

From the checkout root:

```sh
./adapter/scripts/run-local-demo.sh
```

The command builds from the checked-in Cargo lockfiles (Swig client crates are
pinned to `anagrambuild/swig-wallet@3e0411f`), checks the tool versions, then
builds the Veto gate and the test fixtures. It starts a fresh local validator
that clones the official Swig program and the current feature set from devnet
(read-only; set `SWIG_SO` to load a locally built Swig instead), creates
disposable identities, runs the workflow, saves logs under
`verification-logs/`, and removes the temporary identities and ledger on exit.
It submits no public-chain transactions and uses no real funds.

Success ends with `complete-flow-passed`. The run log includes tool versions,
build output, and artifact SHA-256 values. The flow covers approved execution,
target substitution, omitted proof, a genuine loader upgrade, stale-approval
rejection, exact-reviewed-slot enforcement, operator reapproval, proof replay,
nested CPI, policy substitution, and cross-wallet proof substitution.

## Scope

The adapter guards one Swig `SignV2` action per transaction. It binds the
approval to the operator authority, the named agent key, the target program and
its ProgramData slot, the Swig program, config PDA and wallet PDA, and up to
eight approved downstream programs with their slots.

The Solana runtime only lets a program invoke programs among its own
instruction accounts (tested on Agave 4.2.1 by the `sneaky-cpi` fixture in
`--route`: an unlisted callee fails with `MissingAccount`), so every program the
delegated call can reach appears in the call's accounts. Veto reads all of them: each upgradeable-loader program
must be approved at its current deployment slot (finalized programs included,
because anyone can deploy and finalize a new one), native builtins are
allowed, and legacy-loader or loader-v4 programs are refused. Current limits:
approvals add or re-pin programs but cannot yet revoke one, a policy holds at
most eight downstream programs, and routes through legacy-loader programs are
refused. SPL Token itself
runs on the upgradeable loader and is approved at setup. Veto does not
establish what approved code does, replace wallet policy, or constitute a
security audit.

`./adapter/scripts/run-local-demo.sh --route` demonstrates the downstream case:
the agent may call a `payment-router` that forwards to `merchant-pay`; only
merchant-pay is upgraded, so an allowlist of the router alone keeps paying,
while Veto blocks the call until the operator approves the new deployment.

## Payment demo

`./adapter/scripts/run-local-demo.sh --payment` runs the scripted two-wallet
scenario described in [DEMO-SCRIPT.md](DEMO-SCRIPT.md): an ordinary program
allowlist and a Veto-protected Swig wallet, each with a 500 test-token budget,
call the upgradeable `merchant-pay` fixture. The fixture's `drain` feature
builds the malicious v2 that charges the whole balance. Success ends with
`payment-flow-passed`.

The console treats `merchant_pay_v1.so` as the reviewed build. The SHA-256 of
the v1 and v2 builds shipped to the hosted demo is committed in
`adapter/fixtures/merchant-pay/builds.sha256`; `push.sh` and `provision.sh`
refuse other files. These are macOS builds (see the reproducibility note in
`verification-logs/README.md`); update the file when the fixture changes.

## Operator walkthrough

Install and build the pinned browser-wallet client once:

~~~sh
cd adapter/operator
npm ci
npm run build
cd ../..
~~~

Then start the browser demo with a fresh test wallet's address:

~~~sh
OPERATOR_PUBKEY=<wallet-address> ./adapter/scripts/run-local-demo.sh --operator
~~~

Open http://127.0.0.1:4173 in the browser that provides the injected Solana
wallet. The first approval creates the Veto policy, pins the current
merchant-pay deployment, and adds the Veto-bound agent role to the Swig wallet
whose root is your wallet, in one transaction. The page then walks through the
payment, the malicious upgrade, the blocked payment, the redeploy of the
reviewed build, your reapproval, and the resumed payment.

The server binds only to loopback, accepts same-origin POSTs, and prepares and
fee-pays transactions with fresh temporary keys. In this mode it does not hold
the operator key, so it cannot approve a deployment or change the protected
wallet's roles. It holds the agent key, the comparison wallet's root, and the
fixture's upgrade key (standing in for the protocol team). Locally it also
holds the upgrade keys of the Veto gate and test fixtures, which a local
validator needs; on devnet the gate's upgrade key stays off the hosted server.
It refuses to prepare an approval while the deployed code differs from the
reviewed build.

The hosted demo (https://veto-demo.duckdns.org) works the same way when you
connect your own devnet wallet. If you choose the demo operator instead, the
server's test key is the wallet root and signs the approvals, so that run shows
the mechanism, not operator ownership. See [AUTHORITY-MAP.md](AUTHORITY-MAP.md)
for who holds each key.

Checks without a manual wallet:

~~~sh
./adapter/scripts/run-local-demo.sh --operator-smoke            # server-held operator
./adapter/scripts/run-local-demo.sh --external-operator-smoke   # separate operator key
./adapter/scripts/run-local-demo.sh --external-operator-browser # injected test wallet
~~~

The browser check uses an injected provider backed by a fresh key in the
Playwright process; it exercises the wallet API but does not prove
compatibility with a named wallet extension. See
[DISCLOSURE.md](DISCLOSURE.md) for code provenance and licensing, and
[AUTHORITY-MAP.md](AUTHORITY-MAP.md) for who controls which key.
