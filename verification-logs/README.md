# Veto × Swig authorization-boundary verification

Checkout: `/Users/femi/Documents/IdeaCenter/spikes/binarylock-swig-integration`

Branch `binarylock-swig-integration`, Swig source HEAD `3e0411f0c2980b26296903eaa4aa0c2d07869316`. Local SBF tools were installed at `/Users/femi/.local/share/solana/install/active_release/bin/`; they were absent from the prior shell `PATH`, not absent from the machine. Versions: Agave/validator 4.2.1, `cargo-build-sbf` 4.1.0, platform-tools v1.54, Rust 1.89.0.

The unmodified Swig compact-instruction parser's `MAX_ACCOUNTS=254` creates a 6,272-byte SBF frame in `InstructionIterator::parse_next_instruction`; its ordinary v3 artifact failed at runtime with an SBF stack access violation. For the disposable local Swig artifact only, `instructions/src/lib.rs` was temporarily capped at 64 entries, which compiled without the frame warning. This does not alter Swig authorization or SignV2 code. The constant and temporary Swig/assertion program-ID edits were restored after the artifact was built. Thus this was a local build of the inspected Swig source, not a byte-for-byte official release artifact. The tested wallet flows did not exercise high account-count behavior.

## Commands and results

- `cargo test --manifest-path adapter/Cargo.toml --features client --test real_swig_payload -- --nocapture` — **5 passed**. These are host tests using Swig's real client builders; they do not alone establish on-chain enforcement.
- `cargo fmt --manifest-path adapter/Cargo.toml && cargo build --manifest-path adapter/Cargo.toml --features client --bin local_swig_flow` — **passed**.
- `/Users/femi/.local/share/solana/install/active_release/bin/cargo-build-sbf --manifest-path program/Cargo.toml --tools-version v1.54 --arch v3` with temporary `MAX_ACCOUNTS=64` — **passed**, SBF v3. Full output: `build-swig-v3-bounded-accounts.log`.
- The gate, relay fixture, target, and Swig were loaded into a fresh local Agave validator using `--clone-feature-set --url https://api.devnet.solana.com`; the URL was used only to clone the feature set. Human and agent keys were disposable local keys funded by the local validator faucet. No public transactions or real wallets/funds were used. Exact launch arguments and ledger paths are in `local-validator-run.txt`; validator logs are in `validator-rerun.log` and `validator-crosswallet.log`.
- The first full sequence against `http://127.0.0.1:8899` completed with exit 0. An expanded run then added a cross-wallet policy-reuse attempt and exited 1 on its intentional unsafe-result assertion. The latest output in `adversarial-run.log` reaches that final cross-wallet case.

Runner command, executed from the checkout root after the two local airdrops:

```sh
RPC_URL=http://127.0.0.1:8899 \
HUMAN_PATH=adapter/runtime-human.json \
AGENT_PATH=adapter/runtime-agent.json \
GATE_ID=G1uYbHAbmYoS4yom6MmmMfSt78tXNtPHMVfZBQ1jv5J7 \
TARGET_ID=5Nmr67T9jqUh1s9iU5ghABUFt8jMoG3REBZFwiioTMo3 \
RELAY_ID=BXAu5ZWHnGun2XZjUZ9nqwiZ5dNVmofPGYdMC4rx4qLV \
TARGET_KEYPAIR=../binarylock/onchain/target-program/target/deploy/binarylock_target-keypair.json \
TARGET_SO=../binarylock/onchain/target-program/target/sbpfv3-solana-solana/release/binarylock_target.so \
SOLANA_BIN=/Users/femi/.local/share/solana/install/active_release/bin/solana \
adapter/target/debug/local_swig_flow
```

Runtime evidence from that one flow:

1. In the original wallet, an agent-created alternate policy record could not substitute for the policy bound into that Swig role (`Custom(3035)`); target counter remained `0`.
2. Agent reapproval was rejected by the production gate (`Custom(2)`); pinned slot stayed unchanged.
3. Approved delegated execution incremented the target counter to `1`; an alternate target was rejected by the production gate (`Custom(7)`).
4. In the expanded final run, a real local loader upgrade preserved target program ID and changed ProgramData slot `0 → 188`; stale approval was rejected (`Custom(4)`). Human reapproval pinned slot `188`, and agent execution resumed, counter `1 → 2`.
5. A second top-level SignV2 that explicitly reused transaction instruction 0 was rejected by the production instructions-sysvar scan (`Custom(10)`); counter remained `2`.
6. A Swig SignV2 CPI through the relay, reusing instruction 0, failed with Swig `Custom(8)` and left counter at `2`. Swig source at this commit calls `check_stack_height(1, SwigError::Cpi)` in `program/src/actions/sign_v2.rs`; therefore the indirect-call protection is Swig's direct-call restriction, not Veto's top-level scan. This result is specific to the tested Swig build and route.
7. The agent created a second Swig wallet under its own authority, installed a ProgramExec role with the same gate and policy prefix, and executed the approved target using the original policy. This succeeded and moved the shared test counter `2 → 3`. The policy record does not store/check the specific Swig config or wallet PDA, so role-prefix binding alone does not make approval wallet-specific. It used a separate valueless wallet and did not access or spend from the original wallet; this demonstrates scope reuse, not theft of the original wallet's assets.

The Veto SBF adapter and Swig were actual local programs; the loader upgrade and rejection/resumption workflow executed on the local validator. The relay is a purpose-built test fixture. Host tests and validator assertions use local test identities. No public-chain deployment, original-wallet asset bypass, real wallet integration, audit, semantic analysis of upgraded code, user validation, or hackathon eligibility test was performed. Per-wallet approval remains incomplete until the policy binds and checks the intended Swig config/wallet; do not treat the current adapter as a completed guard.

The two unsuccessful build attempts are retained for transparency: `build-swig-v3-stack8192.log` reports a rustc option-name mismatch; `build-swig-v3-bpf-stack8192.log` still reports the 6,272-byte frame. They are not passing evidence.

## Wallet-binding follow-up — September 23, 2026

The previous cross-wallet run proved the gap: after creating a second Swig wallet, the agent installed the same gate/policy-bound role and executed the target using the original policy (counter `2 → 3`). This follow-up added a policy binding to both the Swig config PDA and wallet PDA, enforced at initialization, on every adapter authorization, and during human reapproval. Authorization additionally compares the gate's approved config/wallet to account metas 0 and 1 of the immediately following real Swig `SignV2`. This closes the proof-account substitution shape where the gate is shown the approved wallet but the next `SignV2` comes from another wallet.

### New commands and logs

- `cargo fmt --manifest-path adapter/Cargo.toml` — passed; stable rustfmt emitted warnings that repository-configured nightly-only formatting options were ignored.
- `cargo test --manifest-path adapter/Cargo.toml --features client --test real_swig_payload -- --nocapture` — **6 passed**, including real Swig builder account binding and specific config/wallet mismatch errors. These are host tests, not on-chain enforcement evidence. Output: `host-tests-wallet-binding.log`.
- `/Users/femi/.local/share/solana/install/active_release/bin/cargo-build-sbf --manifest-path adapter/Cargo.toml --tools-version v1.54 --arch v3` — passed. Output: `build-veto-wallet-binding.log`. Built adapter SHA-256: `f4bd05337c918967e6496df3bbaf399f041fc07e7b0c402ac8a299e30c293553`.
- `cargo build --manifest-path adapter/Cargo.toml --features client --bin local_swig_flow` — passed. To make the host driver's compile-time Swig ID match the disposable local Swig artifact, `program/src/lib.rs` and `assertions/src/lib.rs` were temporarily changed from official ID `swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB` to local ID `22zNjxk9cSCWUB7FW85z1v2pQzAkYYobSrWxL2qatgC5`; `program/idl.json` was backed up. A `finally` block restored all three files even on failure. Output: `build-local-flow-wallet-binding.log`.
- Fresh final local validator ledger: `/tmp/veto-swig-wallet-binding-final.DMf8H0`; Agave 4.2.1, `--clone-feature-set --url https://api.devnet.solana.com` (read-only feature-set clone). Exact launch and local-faucet commands: `local-validator-wallet-binding.txt`. Existing disposable local human/agent keys received 10 SOL each from the local validator faucet; no public transactions or real funds were used. The earlier wallet-binding-only run is preserved in `local-flow-wallet-binding.log`; the final run adding a direct-without-adapter attempt is `local-flow-wallet-binding-final.log`.
- The final full-flow command used the same runner environment shown above. It exited **0** and printed `complete-flow-passed`.

### Patched validator evidence

The same production adapter and locally built Swig SBF programs executed the complete local flow. Wrong-config human reapproval failed with adapter `Custom(12)`; wrong-wallet reapproval failed with `Custom(13)`; neither changed the pinned slot. Agent reapproval remained rejected (`Custom(2)`). Approved execution advanced the target counter `0 → 1`; a direct SignV2 submitted with the Veto proof omitted was rejected by Swig (`Custom(3033)`, counter stayed `1`); target substitution was rejected (`Custom(7)`). A genuine local loader upgrade preserved the target ID and moved the slot `0 → 98`; stale approval failed (`Custom(4)`; counter stayed `1`); human reapproval pinned slot `98` and delegated execution resumed (`counter=2`). Top-level proof reuse failed at the production instructions-sysvar scan (`Custom(10)`). The relay CPI attempt remained rejected by Swig's direct-call restriction (`Custom(8)`).

Most importantly, both cross-wallet attempts were submitted and rejected before the target changed: (1) gate and `SignV2` both supplied the agent's second wallet (`Custom(12)` from policy scope mismatch); (2) the gate received the approved original wallet, but the following `SignV2` used the second wallet (`Custom(12)` from comparing the following instruction's account 0 to the approved config). Counter remained `2` for both. This second path exercised the production instructions-sysvar read and account binding on-chain, not just the host helper. Prior unpatched run evidence remains in `adversarial-run.log`, where the first cross-wallet path succeeded and counter moved `2 → 3`; source inspection predicts the new proof-account substitution case would also have passed before the added meta comparison, but that exact stronger variant was not separately rerun against the unpatched binary.

This demonstrates the wallet-binding mechanism locally with an adapter and real Swig source-built SBF artifact under the stated test-only `MAX_ACCOUNTS=64` change. It is not a byte-identical official Swig release artifact. The proof covers the tested direct SignV2 route only; the adapter still constrains one first-hop call and cannot prevent the approved target from making its own CPI. No production wallet integration, broad bypass audit, security review, public deployment, customer validation, or Colosseum eligibility test was performed. The technical authorization gap found in the previous run is now protected in this local adapter; Veto remains a candidate, not a build commitment.

## Full-capacity build and reproducible local demo — September 23, 2026

The earlier `MAX_ACCOUNTS=64` Swig artifact is historical evidence only. The
6,272-byte SBF frame came from two `[MaybeUninit; 254]` arrays in
`InstructionIterator::parse_next_instruction`. The parser now builds
`accounts` and `indexes` as vectors sized to the actual instruction account
count, and `InstructionHolder` owns those vectors. This retains the
`MAX_ACCOUNTS=254` program limit and avoids returning slices into stack-local
arrays.

The packaged runner is `./adapter/scripts/run-local-demo.sh`. It checks Rust
1.91.0, Solana CLI/Agave 4.2.1, `cargo-build-sbf` 4.1.0, platform-tools v1.54,
SBF Rust 1.89.0, and `MAX_ACCOUNTS=254`; it uses locked builds, creates fresh
temporary identities and a fresh validator ledger, then removes those
identities and the ledger. It reads the current devnet feature set to configure
the local validator; all transactions and faucet funds remain local.

The command exited 0. Six host tests passed, then the local validator executed
the complete route: approved action counter `0→1`; policy substitution,
unauthorized reapproval, proof omission and target substitution rejected;
genuine loader upgrade changed ProgramData slot `0→20`; stale approval rejected
with counter still `1`; human reapproval restored action execution and counter
`1→2`; top-level proof replay, relay CPI, cross-wallet policy reuse, and
cross-wallet proof-account substitution all rejected. Output ends
`complete-flow-passed` and `Veto demo passed`.

Artifact SHA-256 values from this run:

- Swig full-capacity SBF v3: `d10f076bd0d9ba636f384568e33202a495020b4a42a62590a608895660239b1c`
- Veto gate SBF v3: `9b68d02fb48ee1cfbefe0711ecaa0fb19c994d25f8b70b97a8c9c27fd7c95c43`
- Relay fixture SBF v3: `0e56bdab3c962de673600ca7451dd7a48cd11d546c7c49d6f964726ef4c3685a`
- Valueless target SBF v3: `3a4ef2a1f7f0ab324041c0c1dc10069e51d84499ad7875ff8f714dccfd2738ce`

The exact run log includes tool versions, compiler output, hashes, and flow
results in `clean-demo-run.log`; validator output is in
`clean-demo-validator.log`. This validates a fresh local runtime setup, not a
cold install on another machine. At this baseline milestone, the operator page
had not yet been built. The first-hop limit, uninspected downstream CPIs,
absent security audit, and unvalidated user/business assumptions remain.

## Operator interface rehearsal — September 23, 2026

The standalone Rust server and responsive browser UI now drive the same local
Veto + Swig transaction route. Start it with
`./adapter/scripts/run-local-demo.sh --operator`; see
[`adapter/DEMO-SCRIPT.md`](../adapter/DEMO-SCRIPT.md) for the walkthrough.
The server binds only to loopback and creates fresh temporary human and agent
identities. Its HTTP actions submit actual local SBF transactions; browser
buttons do not simulate the result in client state.

`./adapter/scripts/run-local-demo.sh --operator-smoke` passed: all five HTTP
actions completed against a fresh local validator, the stale action was
rejected, and the final counter was 2. The interactive browser rehearsal used
Playwright 1.61.0 with Chrome 153.0.8010.53 at viewport widths 320, 375, 414,
768, 1280 and 1440 pixels. Keyboard activation completed execute → redeploy →
stale-call rejection → human reapproval → resumed execution. The check also
verified eight WCAG contrast pairs, keyboard copy feedback, a visible atomic
status announcement, disabled empty controls while status was pending, a
persistent simulated POST failure with no false ledger row, zero enabled
transaction controls while disconnected, zero uncaught page errors, and no
horizontal overflow. At 1280×800 the trust
readout and primary action ended at y=735 and y=698. The captured output is in
`operator-browser-check.log`.

The upgrade redeploys identical fixture bytes, isolating the loader-slot check;
the demo does not analyze program semantics or hashes. The HTTP server signs
with disposable keys, not a user-held wallet. This is local demonstration code,
not a production operator wallet integration. See
[`adapter/DISCLOSURE.md`](../adapter/DISCLOSURE.md) for Swig provenance and
license disclosure.

## External operator and exact-slot verification — September 24, 2026

The policy initialize/reapprove instructions now include the exact deployment
slot reviewed by the operator. The production gate independently reads the
canonical ProgramData slot and returns `Custom(14)` if it differs. A fresh full
validator run executed a genuine loader upgrade from slot `0` to `20`, then
submitted a reapproval carrying reviewed slot `0`; it failed with `Custom(14)`
and the policy remained pinned to slot `0`. The ordinary stale action still
failed with `Custom(4)`, and correct operator reapproval restored execution.
Output: `regression-demo-run.log`. Current Veto gate SBF v3 SHA-256:
`7363f964812e9a57ac15d6d678e712edf46cf16ed81a0b406743b8c9659ff254`.

The loopback server can now prepare and fee-pay an approval transaction whose
other required signer is a configured external operator. It compares a
returned transaction's complete message with the prepared message, preserves
the sponsor signature, verifies all required signatures, and confirms that the
recorded slot equals the reviewed slot. It has no operator secret.

`./adapter/scripts/run-local-demo.sh --external-operator-smoke` exited 0. The
unsigned transaction was rejected, the agent key could not sign it, and the
backend reapproval action returned `external operator signature required`.
The separate operator process initialized the policy, the agent executed,
same-ID redeployment changed slot `0→17`, stale execution failed on-chain with
`Custom(4)` and counter `1`, the operator reapproved slot `17`, and the agent
resumed with counter `2`. Output: `external-operator-smoke-demo-run.log`.

`./adapter/scripts/run-local-demo.sh --external-operator-browser` also exited
0. A fresh operator key existed only in the Playwright process and was exposed
to the page through an injected test-wallet provider; the Rust server received
only its public key and signed transactions. The browser completed initial
wallet approval plus all five workflow actions, retained six evidence rows,
and passed disconnected-state, keyboard, copy, eight contrast-pair, zero-page-
error, and 320/375/414/768/1280/1440-pixel checks. Output:
`external-operator-browser-demo-run.log`.

This proves the browser/server signing boundary with an injected test provider,
not compatibility with Phantom or another named production extension. The
server still owns disposable setup, Swig-root, agent, fee-payer, and target-
upgrade keys for the local fixture; it cannot initialize or reapprove the
existing external-authority Veto policy. No public deployment, real funds,
semantic upgrade analysis, downstream-CPI protection, durable history,
independent audit, or customer evidence was added. The bundled
`@solana/web3.js` 1.99.0 dependency currently carries transitive npm audit
advisories; the browser bridge should be minimized or migrated before any
production claim.

## Manual browser-wallet run — September 24, 2026

The user used Phantom and supplied public operator address
`FbJLh2PYzDZ6rb9uVkM4vk7TnSgqizeUXHYeN2HtKBZ4` and manually completed the
browser workflow against a fresh disposable validator. The final status endpoint
reported `phase=resumed`, `externalOperator=true`, approved and current slot
`305`, and counter `2`. The latest submitted agent action moved the counter
`1→2` and returned signature
`cbmWBY35h9QnxNW1WGgsYToSk71i8fgixCZCiJMuBtL2kCB7B9Tv483EsuJAQRQCsqXrUHASeKqATWu7BKERdH8`.
The captured response is `manual-wallet-final-status.json`.

This establishes that Phantom could complete the browser-injected local signing
workflow. After capture, Ctrl-C triggered the runner cleanup and ports 4173 and
8899 were confirmed closed.

## Authority fixes (F1–F3) — 2026-09-24

Fixes for the findings in [`adapter/AUTHORITY-MAP.md`](../adapter/AUTHORITY-MAP.md), verified on fresh local validators with the same toolchain and full-capacity Swig artifact (`d10f076b…9b1c`, unchanged). New Veto gate SHA-256: `a0e1f317f19a01012be966de80365d3507a3ef49eebb65a95a67e4ade87cd255`.

- **F1 agent identity:** the policy now stores an agent key (bytes 169..201, `POLICY_LEN=201`) and `authorize` requires that key as a signer. Fresh run: a funded stranger submitting as its own agent was rejected `Custom(16)`; naming the real agent without its signature was rejected `Custom(15)`; counter stayed `0`.
- **F2 operator-owned wallet:** the operator server now creates the Swig with the operator's key as root (`All`); the server only pays. The Veto `ProgramExec` role is added with the operator signing as role 0, so the server cannot add, remove or change roles.
- **F3 front-running:** `initialize` requires the policy account's own signature (`Custom(17)` otherwise), and the operator's first approval transaction creates the policy, initializes it and adds the Swig role atomically. Fresh run: a stranger initializing an unclaimed policy was rejected `Custom(17)` and the account stayed uninitialized.

Commands (exit 0 each): `./adapter/scripts/run-local-demo.sh` → `complete-flow-passed`, saved as [`2026-09-24-authority-fix-full-flow.log`](2026-09-24-authority-fix-full-flow.log); `--external-operator-smoke` → `external-operator-smoke-passed`, saved as [`2026-09-24-authority-fix-external-operator-smoke.log`](2026-09-24-authority-fix-external-operator-smoke.log); `--external-operator-browser` → `external-operator-browser-passed` (6 actions, counter 2, 320–1440 px, 0 page errors), saved as [`2026-09-24-authority-fix-external-operator-browser.log`](2026-09-24-authority-fix-external-operator-browser.log). These runs overwrote the earlier same-day `regression-demo-run.log`, `external-operator-smoke-demo-run.log` and `external-operator-browser-demo-run.log`, which recorded the pre-fix code. All earlier adversarial cases still pass. The server still holds the target-upgrade key (it plays the third-party protocol team) and the fee-payer key. Phantom was not re-run manually after these changes.

## Test-token payment with a malicious upgrade — 2026-09-24

`./adapter/scripts/run-local-demo.sh --payment` (exit 0, `payment-flow-passed`), saved as [`2026-09-24-payment-flow.log`](2026-09-24-payment-flow.log). Fresh local validator, disposable keys, a valueless 6-decimal test mint.

Setup: two Swig wallets, both with the operator key as root and a 500-token budget. The agent role on each is `Program(merchant-pay)` + `TokenLimit(mint, 500)`. On the **plain** wallet the role is the agent's Ed25519 key (an ordinary program allowlist). On the **Veto** wallet it is the `ProgramExec` role bound to a Veto policy that names the operator, the agent, and the reviewed merchant-pay deployment slot. The merchant-pay upgrade authority is a separate "protocol team" key.

| Step | Plain allowlist | Veto |
|---|---|---|
| v1 (reviewed build) pays 10 | 500 → 490 | 500 → 490 |
| Protocol key upgrades to v2 (same program ID, slot 0 → 19, different code hash) | — | — |
| Agent requests a 10-token payment | **charged 490** (balance 0) | **rejected `Custom(4)`**, balance 490 unchanged |
| Protocol redeploys the reviewed v1 build (slot 28, hash matches reviewed build) | — | still rejected `Custom(4)` until review |
| Agent tries to reapprove | — | rejected `Custom(2)` |
| Operator reapproves slot 28; agent pays 10 | — | 490 → 480 |

Merchant-pay builds (`adapter/fixtures/merchant-pay`, same source; `--features drain` builds v2): v1 file SHA-256 `82e1121a…0aaa`, v2 `706f7cd9…c9da`. The code hash printed by the flow is SHA-256 of the executable with trailing zero padding removed, computed identically for the file and for the deployed ProgramData bytes after loader metadata (v1 `4d728ca2…0045`, v2 `abb80f46…991d`). This is a local reviewed-build comparison, not `solana-verify` output or a reproducible-build attestation.

Limits: v2 is a deliberately malicious fixture written for this demo, not a replayed real incident. Swig's `TokenLimit` bounded the plain wallet's loss to its remaining budget; Veto's contribution is that unreviewed code could not spend any of it. The Veto wallet's protection covers the direct first-hop call; merchant-pay's own CPI to SPL Token is inside the approved code.

## Operator console on the payment scenario — 2026-09-24

The operator server and page now run the two-wallet payment scenario instead of the counter. The protected wallet's Swig root is the operator; the first operator signature creates, initializes and binds the policy atomically; the console refuses to prepare an approval while the deployed code hash differs from the reviewed build (HTTP 409). Final-code runs (exit 0 each): regression [`2026-09-24-ui-payment-regression.log`](2026-09-24-ui-payment-regression.log), payment [`2026-09-24-ui-payment-flow.log`](2026-09-24-ui-payment-flow.log), external-operator smoke [`2026-09-24-ui-payment-external-operator-smoke.log`](2026-09-24-ui-payment-external-operator-smoke.log) (unsigned approval rejected, agent cannot sign, v2 approval refused, plain wallet 0.00 vs Veto 490.00, backend reapproval refused, resume 480.00), and browser [`2026-09-24-ui-payment-external-operator-browser.log`](2026-09-24-ui-payment-external-operator-browser.log) (7 actions, 320–1440 px, no overflow, 0 page errors). Screenshots: [`2026-09-24-ui-payment-blocked.png`](2026-09-24-ui-payment-blocked.png), [`2026-09-24-ui-payment-resumed.png`](2026-09-24-ui-payment-resumed.png). Not yet re-run manually with Phantom.

## Official Swig (devnet binary) compatibility — 2026-09-24

`SWIG_SOURCE=devnet-clone` makes the runner load Swig with `--clone-upgradeable-program swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB` from devnet instead of the locally modified build. A separate check confirmed the cloned code hash equals devnet's (`1da1c420a476f82d5d54f33cc16ffeeebabbaae37a2693a214767453a433f93b`, SHA-256 of ProgramData after metadata with trailing zeros removed) and differs from the local build (`54bc5682…7432`). Against the official binary, the payment flow passed ([`2026-09-24-official-swig-clone-payment.log`](2026-09-24-official-swig-clone-payment.log)) and the full attack regression passed with identical error codes ([`2026-09-24-official-swig-clone-regression.log`](2026-09-24-official-swig-clone-regression.log)). The devnet demo therefore uses the official Swig program; the local Swig parser change is not needed for it. This is a local-validator run of the devnet binary, not a devnet transaction.

## Route-wide pinning — 2026-09-25

The gate now checks every program the delegated call can reach, not only the first hop. Basis: the agave runtime rejects a CPI whose callee is not among the caller's instruction accounts (`solana-program-runtime` 3.1.12, `invoke_context.rs` 403–418), so the delegated call's accounts contain every reachable program. Programs are classified by owning loader: upgradeable-loader programs (including finalized ones) must be approved at their current slot; native builtins pass; legacy-loader and loader-v4 programs are refused. New policy layout: 522 bytes (header, count, up to 8 route entries). New instruction `3` approves a downstream program (operator-signed). SPL Token is on the upgradeable loader (finalized) on devnet and mainnet and is approved at setup. Gate SHA-256: `acc3073f9c3faeda30d10f4053f1658665ae384a30a1be37aa94961f382193ad`.

`--route` ([`2026-09-25-route-flow.log`](2026-09-25-route-flow.log)): unapproved downstream program rejected `Custom(19)`; agent approval of a downstream program rejected `Custom(2)`; missing downstream ProgramData rejected `Custom(18)`; a freshly deployed and finalized copy of the malicious v2 routed through the router rejected `Custom(19)` with no balance change; after only the downstream merchant-pay was upgraded (router slot unchanged), the router-only allowlist wallet was charged 490 while Veto rejected `Custom(20)`; still blocked after the reviewed build was redeployed; resumed at 480 after operator approval. Same flow against the official Swig devnet binary: [`2026-09-25-route-official-swig-clone.log`](2026-09-25-route-official-swig-clone.log). All earlier modes re-run on this gate: regression, payment, external-operator smoke, browser (logs `2026-09-25-route-gate-*`). Host tests 7/7 (new: inner-account extraction from a real Swig payload).

**Runtime rule tested (2026-09-25).** `--route` now first checks the rule route pinning depends on, with the `sneaky-cpi` fixture: merchant-pay's ID is placed in the transaction by one instruction, and a second instruction tries to invoke it without listing it as an account → `InstructionError(1, MissingAccount)`. Control: the same call with merchant-pay listed passes the runtime check and fails inside merchant-pay (`NotEnoughAccountKeys`). Passed with the local Swig build and the official Swig clone on Agave 4.2.1 ([`2026-09-25-route-flow-runtime-rule.log`](2026-09-25-route-flow-runtime-rule.log), [`…-official-swig.log`](2026-09-25-route-flow-runtime-rule-official-swig.log)). Browser check re-run on the deployed page text: [`2026-09-25-deployed-page-browser.log`](2026-09-25-deployed-page-browser.log).

## Standalone repository — 2026-09-25

This repository was assembled from the checkout above. Entries earlier in this
file reference that checkout's paths (for example Swig's `program/` and
`test-program-authority/`), which are not part of this repository.

From this repository, with Swig's client crates pinned to
`anagrambuild/swig-wallet@3e0411f` and the official Swig program cloned from
devnet, all modes passed (exit 0): attack regression (including the new relay
fixture: nested CPI rejected by Swig `Custom(8)`), `--payment`, `--route`
(runtime rule, finalized-program and downstream-upgrade cases),
`--external-operator-smoke`, `--external-operator-browser`, and
`--operator-smoke`. Logs: `2026-09-25-standalone-*`. The standalone gate build
is byte-identical to the gate deployed on devnet at
`4okceHnZABKcqadXLK57mkU87c4GAUKNsunr5LHHShJq` (SHA-256 `acc3073f…93ad`).

## External review fixes — 2026-09-26

An external read-only review raised findings on Veto and on the Swig code in the earlier checkout. The Veto findings were fixed:

- **Approving unreviewed code.** Approval paths relied on the page phase; the server did not re-check the code hash, and read slot and hash separately. Every approval path (wallet-signed and server-signed) now reads slot and hash from one ProgramData snapshot and refuses on a mismatch; the page disables approval on a mismatch. Regression: `--external-operator-smoke` upgrades merchant-pay out-of-band while the phase is "fixed" and requires the approval request to be refused (`out-of-band-unreviewed-approval-refused`).
- **RPC URL in public status.** `/api/status` now returns a network name, not the RPC URL.
- **Slow requests.** 10-second total request deadline and at most 64 concurrent connections (503 beyond); `--operator-smoke` checks that a client trickling one byte every 3 seconds is cut off (`slow-client-cut-off-after=18s`, versus the 60 s it would otherwise hold).
- **Opaque signing.** The wallet bridge and standalone signer refuse approvals in which the operator pays the fee, is writable, or is not a required signer (unit tests in `external_operator_signer`).
- **No CI.** `.github/workflows/ci.yml` runs host tests, client builds, a bundle-freshness check and SBF builds.

Logs: `2026-09-26-review-fix-*`. The Swig findings from the same review (ProgramScope cache, SDK signers and odometers, CLI key storage, zero-window limits, session replacement policy) concern upstream Swig code, which Veto neither vendors nor uses in those paths.
