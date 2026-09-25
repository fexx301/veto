# Veto on Solana devnet

Recorded 2026-09-24. All transactions are on public **devnet** with valueless
test tokens. Swig is the **official** devnet program; it was not redeployed.

## Programs

| Program | Address | Notes |
|---|---|---|
| Veto gate | [`AE6zACTjyi…`](https://explorer.solana.com/address/AE6zACTjyiYwmNYyVbkh75aBzJ5YfQMHuxLSbMYBrxD1?cluster=devnet) | SHA-256 of build `a0e1f317…d255`; upgrade authority still held by the deployer (not yet finalized) |
| merchant-pay (demo fixture) | [`Ejm686FUtJ…`](https://explorer.solana.com/address/Ejm686FUtJiuuYdWPMT4JNVaZopMFb8pYamfXJ9jPa2N?cluster=devnet) | Upgrade authority: separate "protocol team" key [`DAgmS2n6RY…`](https://explorer.solana.com/address/DAgmS2n6RYKua3hFt8mkW6m6Fm3z9e65xXc82zXAVMah?cluster=devnet) |
| Swig (official) | [`swigypWHEk…`](https://explorer.solana.com/address/swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB?cluster=devnet) | Deployed by the Swig team; code hash `1da1c420…f93b` |

## Scenario run (server-held operator key)

Accounts: Veto policy [`JDLYwXuH6G…`](https://explorer.solana.com/address/JDLYwXuH6G43sR32u7ZFrjbfnzWyf3WbdTXTbbnVvR4N?cluster=devnet), protected Swig
config [`GL1oM1Q9Fv…`](https://explorer.solana.com/address/GL1oM1Q9FvRTZRo98QZrRZAZLFaBeLhAaALtLmfg115e?cluster=devnet), plain-allowlist Swig
config [`A96ucCemna…`](https://explorer.solana.com/address/A96ucCemnaAF2ouHmpCPx5H1QQVxcHFUS56LUWCVWY4W?cluster=devnet), agent [`CahMVvB45Z…`](https://explorer.solana.com/address/CahMVvB45Zgmar2LSCYucwzvRoX3ca5nDcz37GYYaiDs?cluster=devnet),
test mint [`6BKxrEJbKn…`](https://explorer.solana.com/address/6BKxrEJbKnvxhyK9feqCZjKbMxd4FdSGHFmsJgh3BE23?cluster=devnet). Each wallet started with 500 test tokens.

| Step | Transaction | Result |
|---|---|---|
| Agent pays 10 via Veto (reviewed v1) | [`4ZUDF2hi6j…`](https://explorer.solana.com/tx/4ZUDF2hi6jQPtQM694tLi1o7k9HYT2wdCoH973DxgcTwcSyGoBh8trAprqWJdURHa2BrrjQBkqWDutjd7a68YCkP?cluster=devnet) | Veto wallet 500 → 490 |
| Agent pays 10 via plain allowlist | [`47YPSAcE2N…`](https://explorer.solana.com/tx/47YPSAcE2NaGEp9dQEiQC5RvHHVtBfXgLdfCeurWM5jkNMpSoBj4PNkZs6PX6gSzZ3fAnofwnafFd8sZvnfwtRuG?cluster=devnet) | Plain wallet 500 → 490 |
| Protocol key upgrades merchant-pay to v2 | [`3fcsqY3YDW…`](https://explorer.solana.com/tx/3fcsqY3YDWbMUWRnQKuFx9TTjzcXhpdxgNyNxKvt73uuxSW6RRAoZSDEpRxLqAHrG7RXbUm74MGzVTAHgdYM87qQ?cluster=devnet) | Same program ID; slot 503516837 → new slot |
| Agent requests 10 via plain allowlist | [`5eGo2ZuBbu…`](https://explorer.solana.com/tx/5eGo2ZuBbuKQd7QAeKYpznoeiwaXs7q3b4yJdCM1i6Finfar4hNBnd7V97EsXM6yUUiJQdxojbJUP199mdUUruGS?cluster=devnet) | Log: `merchant-pay v2 requested=10000000 charged=490000000`; wallet 490 → 0 |
| Agent requests 10 via Veto | [`3362HyVBpA…`](https://explorer.solana.com/tx/3362HyVBpAN9f8dKhU57hmR6kM4X4NbzrRHRUA2N6ZAdmw8CzA1oESGtWaMc8e9Mr9Wai3sTKW7GKtwYiyzwTR5j?cluster=devnet) | **Failed** in Veto: `custom program error: 0x4` (deployment changed); balance stays 490 |
| Protocol redeploys the reviewed v1 build | [`4KUU74cM5j…`](https://explorer.solana.com/tx/4KUU74cM5jvapYgPkN3bkeezghwUYUJvNnGA6YRX4LGpBcmdiTBBv3rVpMiQf8oUGdvADEnt8ss76qwUxTZd1eW4?cluster=devnet) | Code hash matches reviewed build; Veto still paused |
| Operator reapproves the new slot | [`3x6goZoxGd…`](https://explorer.solana.com/tx/3x6goZoxGdtmboqLCgT7PCTndgnqpi1g7EjuT2ZbAsfn9j5rMKHotW55kZHYXnJWdQr53kP7yncvuV1U3zDtjS16?cluster=devnet) | Policy slot updated |
| Agent pays 10 via Veto | [`23R9rNMp97…`](https://explorer.solana.com/tx/23R9rNMp971YmLbcAzU7bV2Pma4nxtDUTyontkpsWeuBGivh9K5knRLLLJ5kwZdtpmGGH5Hk5gEP6qvunZmg8UWf?cluster=devnet) | Veto wallet 490 → 480 |

Full server responses: [`2026-09-24-devnet-scenario.jsonl`](2026-09-24-devnet-scenario.jsonl).

## Limits

- In this run the operator was the server's deployer key, not a browser
  wallet. A Phantom-signed devnet run is still to be recorded.
- merchant-pay v2 is a deliberately malicious demo fixture, not a real
  protocol. The plain wallet's loss was capped by Swig's token limit.
- The Veto gate is not yet immutable; see `deploy-devnet.sh --finalize`.

## Hosted mode (two simulated visitors, demo operator) — 2026-09-24

`HOSTED=1` served the console against devnet on loopback. Visitor A started a run with the demo operator and completed all six steps (plain 490 → 0 under v2, Veto 490 held, then 480 after reapproval). Visitor B, with no run cookie, was refused when trying to act on A's run ("this run belongs to another visitor") and when trying to start a new run while A's was active ("another visitor is running the demo"). During A's 20-second devnet upgrade B's status request returned in about 20 ms from the cache (`busy: true`). After A's run finished, B's status reported `canStart: true`. A stale-cache issue found in this test (actions did not refresh the cache) was fixed afterwards.

## Route-pinning gate and first public-URL run — 2026-09-25

The gate with route-wide pinning (policy layout changed) is deployed at a new address, [`4okceHnZ…ShJq`](https://explorer.solana.com/address/4okceHnZABKcqadXLK57mkU87c4GAUKNsunr5LHHShJq?cluster=devnet) (SHA-256 `acc3073f…93ad`, upgrade authority still held by the deployer, not finalized). The earlier gate `AE6zACTj…rxD1` remains deployed so the links above stay valid. The hosted console at https://veto-demo.duckdns.org now uses the new gate.

First complete run on the public URL (demo operator, cookie-owned run), full server responses in [`2026-09-25-hosted-public-run.jsonl`](2026-09-25-hosted-public-run.jsonl):

| Step | Transaction | Result |
|---|---|---|
| Veto wallet pays 10 (v1) | [`rTsGT1JAS7…`](https://explorer.solana.com/tx/rTsGT1JAS755Rdj5Njx63NcNuLpgoTHMaQWdkvuxWiugkBPJoMej6AfRBJYoNkCtLAUXXrrJncF9aHy6gp1FJph?cluster=devnet) | 500 → 490 |
| Protocol upgrades merchant-pay to v2 | [`4bxdPqZwqp…`](https://explorer.solana.com/tx/4bxdPqZwqpLkQxwBNnkV1wMFTuugEzSPFeCHUpgSx8ZV6ruStqTrGTHpSEJHNzSqTBsw4Fnr97fQ9RuSAfwJQ8jW?cluster=devnet) | same program ID, new slot |
| Plain allowlist requests 10 | [`2BtjExFUmL…`](https://explorer.solana.com/tx/2BtjExFUmLn3AFqb1wqtc7v6AYzAXjtufpxZWvXCxJq5zpC2UYzmN3qs9A8Fq6A6dmhxeF8VYHm4Es5EN2hoDrCc?cluster=devnet) | charged 490 |
| Veto wallet requests 10 | [`32MemFm5Fd…`](https://explorer.solana.com/tx/32MemFm5FdRMHGZ1xgZLEBdg7zuy5x3AvhFCMXuPx9s9ZEzUFTSi72UMW1PdArTytG9k1gh9L76YxjNuq7SsbVj5?cluster=devnet) | failed in `4okce…`: `custom program error: 0x4`; 490 kept |
| Reviewed build redeployed | [`4iSu5v8rA3…`](https://explorer.solana.com/tx/4iSu5v8rA3YeaaN2eBBhmzBDKZUrsx3xJhV2kCUnvSkmpkGbN6GgPHcGcDuALMZGEpXmSvRZCo4E5LFMfK1HHUhn?cluster=devnet) | still paused |
| Operator reapproves | [`5s1LCu15bD…`](https://explorer.solana.com/tx/5s1LCu15bDpnqpqscB5Wk5Ftdp6trfxnEzPzC28NT683tqVg4jhAtcGCoRxLfaPHwndA9zx2jGxWcMcPJqNoD6zE?cluster=devnet) | new slot pinned |
| Veto wallet pays 10 | [`4PVqgpj6Vv…`](https://explorer.solana.com/tx/4PVqgpj6Vv2cy7zx6Nvgkr4i7WL7TJdscCqQrhEVVsm9GcC1wrKG2CVjUc4x3yFdsfZzP4afQgpMmYwKub9vEMZW?cluster=devnet) | 490 → 480 |

The downstream route demo (`--route`) has run locally and against the official Swig binary, not yet on devnet. A Phantom-signed run on the public URL is still pending.
