#!/usr/bin/env python3
"""Summarizes research/upgrade_frequency.py output.

Usage: python3 research/summarize_upgrades.py <out-dir>/upgrade-frequency.json
"""

import json
import statistics
import sys
import time

data = json.load(open(sys.argv[1]))
rows = data["results"]
ok = [r for r in rows if "error" not in r and r.get("exists", True)]
missing = [r for r in rows if r.get("exists") is False]
errors = [r for r in rows if "error" in r]
upgradeable = [r for r in ok if r.get("upgradeable") is True]
frozen = [r for r in ok if r.get("upgradeable") is False]
other = [r for r in ok if r.get("upgradeable") is None]
single = [r for r in upgradeable if r.get("authority_kind") == "single key"]
pda = [r for r in upgradeable if r.get("authority_kind", "").startswith("PDA")]
u90 = [r["upgrades_90d"] for r in upgradeable]
u180 = [r["upgrades_180d"] for r in upgradeable]
upgraded_180 = [r for r in upgradeable if r["upgrades_180d"] > 0]
upgraded_90 = [r for r in upgradeable if r["upgrades_90d"] > 0]

print(f"generated_at {time.strftime('%Y-%m-%d %H:%M UTC', time.gmtime(data['generated_at']))}; rpc {data['rpc']}")
print(f"programs: {len(rows)} total, {len(ok)} measured, {len(errors)} errors, {len(missing)} not found")
print(f"upgradeable now: {len(upgradeable)}/{len(ok)}; immutable: {len(frozen)}; other loader: {len(other)}")
print(f"upgrade authority: single key {len(single)}, PDA (multisig/governance) {len(pda)}")
print(f"upgraded at least once: last 90d {len(upgraded_90)}/{len(upgradeable)}, last 180d {len(upgraded_180)}/{len(upgradeable)}")
print(f"upgrades per upgradeable program, 180d: total {sum(u180)}, median {statistics.median(u180)}, mean {statistics.mean(u180):.1f}")
active = [r["upgrades_180d"] for r in upgraded_180]
if active:
    print(f"among programs upgraded in 180d: median {statistics.median(active)} upgrades ≈ one every {180 / statistics.median(active):.0f} days")
buckets = {"0": 0, "1-2": 0, "3-6 (≈monthly)": 0, "7-26 (≈weekly)": 0, "27+ (more than weekly)": 0}
for n in u180:
    key = "0" if n == 0 else "1-2" if n <= 2 else "3-6 (≈monthly)" if n <= 6 else "7-26 (≈weekly)" if n <= 26 else "27+ (more than weekly)"
    buckets[key] += 1
print("180d upgrade-count distribution:", buckets)
single_upgraded = [r for r in single if r["upgrades_180d"] > 0]
print(f"single-key authority AND upgraded in 180d: {len(single_upgraded)}")
print("\nmost upgraded (180d):")
for r in sorted(upgradeable, key=lambda r: -r["upgrades_180d"])[:10]:
    print(f"  {r['upgrades_180d']:>3} / 90d {r['upgrades_90d']:>3}  {r['authority_kind']:<26} {r['label']}  {r['program']}")
print("\ncurated first-hop programs:")
for r in rows:
    if "curated" in r["label"]:
        print(f"  {r['label']:<48} upgradeable={r.get('upgradeable')} 90d={r.get('upgrades_90d')} 180d={r.get('upgrades_180d')} auth={r.get('authority_kind')} {'ERROR' if 'error' in r else ''}")
if frozen or other:
    print("\nimmutable / other loaders:")
    for r in frozen + other:
        print(f"  {r['label']}  {r.get('loader')}")
if missing:
    print("\nnot found on mainnet:", [r["label"] for r in missing])
if errors:
    print("\nerrors:", [r["label"] for r in errors])
