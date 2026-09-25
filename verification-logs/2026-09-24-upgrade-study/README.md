# Upgrade frequency of programs agents route funds through

Measured 2026-09-24 (window ends 20:38 UTC) with `research/upgrade_frequency.py`
against the public mainnet RPC. Read-only.

**Set:** the 106 programs Jupiter's router lists
(`jupiter-program-id-to-label.json`, fetched from
`https://lite-api.jup.ag/swap/v1/program-id-to-label`) plus 11 curated
first-hop programs (Jupiter v6, Jupiter DCA, Kamino Lend, marginfi v2,
Save/Solend, Drift v2, Marinade, Swig, Squads v4, Token-2022, Solana
Subscriptions & Allowances; Jupiter Perps is in both lists). 117 programs.

**Method:** loader state and upgrade authority from each ProgramData account.
An upgrade = a successful `bpf-upgradeable-loader` `upgrade` instruction
(top-level or inner) targeting that ProgramData. The main pass fetched every
transaction touching the ProgramData in the last 180 days. Seven programs failed
first (network errors; version-1 transactions); they were re-measured by
fetching only transactions that touch both the ProgramData and the current
upgrade authority. That method reproduced the four results already obtained
the slow way (Deriverse 53/10, DexLab 0/0, Jupiter v6 23/10, Marinade 1/1). It
would miss upgrades signed by an authority rotated within the window. The
detector was validated on devnet merchant-pay: 9 of 9 known upgrades found,
22 unrelated transactions ignored.

**Authority kind:** "single key" = the authority address is on the ed25519
curve (an ordinary keypair, hot or cold). "PDA" = off-curve, i.e. controlled
by a program, typically a multisig or governance program.

## Results (`summary.txt`)

- 115 of 117 can still be upgraded; immutable: Orca V1 (legacy loader) and Squads v4.
- 67 of 115 were upgraded in the last 180 days (52 in the last 90); 712 upgrades in total.
- Distribution over 180 days: 48 none, 18 once or twice, 22 roughly monthly (3–6),
  18 roughly weekly (7–26), 9 more than weekly (27+).
- Among programs upgraded at all, median 5 upgrades in 180 days (about one every 36 days).
- 35 have a single-key upgrade authority; 19 of those were upgraded in the last 180 days.
- Jupiter Aggregator v6: 23 upgrades in 180 days (10 in 90), multisig/program authority.
  Kamino Lend 9, Swig 7, Drift 6, marginfi 6, Solana Subscriptions & Allowances 3.

Limits: a snapshot of one list at one time; Jupiter's router list is not a list
of programs agents call directly; upgrade counts say nothing about whether any
upgrade was malicious (none is known to be).
