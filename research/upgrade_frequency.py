#!/usr/bin/env python3
"""Upgrade history of Solana programs that agents route funds through.

Read-only. For each program: which loader owns it, whether it can still be
upgraded, whether its upgrade authority is a single key (on the ed25519 curve)
or a program-derived address (typically a multisig or governance program), and
how many loader `upgrade` instructions targeted its ProgramData account in the
last 90 and 180 days.

Usage:
  python3 research/upgrade_frequency.py <jupiter-labels.json> <out-dir>

`jupiter-labels.json` is the response of
https://lite-api.jup.ag/swap/v1/program-id-to-label (programs Jupiter routes
swaps through). A short curated list of first-hop programs is added; those IDs
are verified to exist on-chain, and their labels are marked "curated".
"""

import base64
import json
import sys
import time
import urllib.error
import urllib.request

RPC = "https://api.mainnet-beta.solana.com"
UPGRADEABLE_LOADER = "BPFLoaderUpgradeab1e11111111111111111111111"
IMMUTABLE_LOADERS = {
    "BPFLoader2111111111111111111111111111111111": "bpf-loader-v2 (immutable)",
    "BPFLoader1111111111111111111111111111111111": "bpf-loader-v1 (immutable)",
}
LOADER_V4 = "LoaderV411111111111111111111111111111111111"
DAY = 86_400
MIN_INTERVAL = 0.25  # seconds between RPC calls, well under public limits

CURATED = {
    "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4": "Jupiter Aggregator v6 (curated)",
    "DCA265Vj8a9CEuX1eb1LWRnDT7uK6q1xMipnNyatn23M": "Jupiter DCA (curated)",
    "PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu": "Jupiter Perps (curated)",
    "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD": "Kamino Lend (curated)",
    "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA": "marginfi v2 (curated)",
    "So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo": "Save / Solend (curated)",
    "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH": "Drift v2 (curated)",
    "MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD": "Marinade (curated)",
    "swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB": "Swig wallet (curated)",
    "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf": "Squads v4 (curated)",
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb": "Token-2022 (curated)",
    "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44": "Solana Subscriptions & Allowances (curated)",
}

_last_call = 0.0


def rpc(method, params, retries=6):
    global _last_call
    for attempt in range(retries):
        wait = MIN_INTERVAL - (time.time() - _last_call)
        if wait > 0:
            time.sleep(wait)
        _last_call = time.time()
        body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
        request = urllib.request.Request(RPC, body, {"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.load(response)
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
            time.sleep(2 ** attempt)
            last_error = error
            continue
        if "error" in payload:
            if payload["error"].get("code") in (-32429, 429, -32005):
                time.sleep(2 ** attempt)
                last_error = payload["error"]
                continue
            raise RuntimeError(f"{method}: {payload['error']}")
        return payload["result"]
    raise RuntimeError(f"{method}: gave up after retries: {last_error}")


# --- base58 and the ed25519 curve check used to tell keys from PDAs ---------
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58encode(raw):
    number = int.from_bytes(raw, "big")
    out = ""
    while number:
        number, remainder = divmod(number, 58)
        out = ALPHABET[remainder] + out
    return "1" * (len(raw) - len(raw.lstrip(b"\0"))) + out


def b58decode(text):
    number = 0
    for char in text:
        number = number * 58 + ALPHABET.index(char)
    raw = number.to_bytes(32, "big")
    return raw


P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P


def on_curve(key_bytes):
    """True if the 32 bytes decompress to an ed25519 point (a normal key);
    false for program-derived addresses, which are deliberately off-curve."""
    y = int.from_bytes(key_bytes, "little") & ((1 << 255) - 1)
    if y >= P:
        return False
    u = (y * y - 1) % P
    v = (D * y * y + 1) % P
    x2 = u * pow(v, P - 2, P) % P
    return x2 == 0 or pow(x2, (P - 1) // 2, P) == 1


# -----------------------------------------------------------------------------
def program_state(program):
    info = rpc("getAccountInfo", [program, {"encoding": "jsonParsed"}])["value"]
    if info is None:
        return {"exists": False}
    owner = info["owner"]
    state = {"exists": True, "executable": info["executable"], "owner": owner}
    if owner in IMMUTABLE_LOADERS:
        state.update(loader=IMMUTABLE_LOADERS[owner], upgradeable=False)
        return state
    if owner == LOADER_V4:
        state.update(loader="loader-v4", upgradeable=None)
        return state
    if owner != UPGRADEABLE_LOADER:
        state.update(loader=f"other ({owner})", upgradeable=None)
        return state
    programdata = info["data"]["parsed"]["info"]["programData"]
    raw = rpc("getAccountInfo", [programdata, {"encoding": "base64", "dataSlice": {"offset": 0, "length": 45}}])["value"]
    data = base64.b64decode(raw["data"][0])
    slot = int.from_bytes(data[4:12], "little")
    has_authority = data[12] == 1
    authority = b58encode(data[13:45]) if has_authority else None
    state.update(
        loader="bpf-loader-upgradeable",
        programdata=programdata,
        last_deploy_slot=slot,
        upgradeable=has_authority,
        authority=authority,
        authority_kind=(None if not has_authority else ("single key" if on_curve(data[13:45]) else "PDA (multisig/governance)")),
    )
    return state


def upgrade_history(programdata, since):
    """Loader `upgrade` instructions targeting this ProgramData since `since`."""
    signatures, before = [], None
    for _ in range(4):
        params = {"limit": 1000}
        if before:
            params["before"] = before
        page = rpc("getSignaturesForAddress", [programdata, params])
        if not page:
            break
        signatures.extend(page)
        before = page[-1]["signature"]
        if len(page) < 1000 or (page[-1].get("blockTime") or 0) < since:
            break
    upgrades = []
    for entry in signatures:
        if entry.get("err") is not None or (entry.get("blockTime") or 0) < since:
            continue
        tx = rpc("getTransaction", [entry["signature"], {"encoding": "jsonParsed", "maxSupportedTransactionVersion": 1}])
        if tx is None:
            continue
        message = tx["transaction"]["message"]
        instructions = list(message["instructions"])
        for inner in (tx.get("meta") or {}).get("innerInstructions") or []:
            instructions.extend(inner["instructions"])
        for ix in instructions:
            parsed = ix.get("parsed") if isinstance(ix, dict) else None
            if ix.get("program") == "bpf-upgradeable-loader" and isinstance(parsed, dict) and parsed.get("type") == "upgrade":
                if parsed.get("info", {}).get("programDataAccount") == programdata:
                    outer = sorted({i.get("programId") for i in message["instructions"]})
                    upgrades.append({"signature": entry["signature"], "time": entry["blockTime"], "outer_programs": outer})
                    break
    return upgrades, len(signatures)


def signatures_since(address, since, max_pages=20):
    found, before = [], None
    for _ in range(max_pages):
        params = {"limit": 1000}
        if before:
            params["before"] = before
        page = rpc("getSignaturesForAddress", [address, params])
        if not page:
            break
        found.extend(page)
        before = page[-1]["signature"]
        if len(page) < 1000 or (page[-1].get("blockTime") or 0) < since:
            break
    return [entry for entry in found if (entry.get("blockTime") or 0) >= since and entry.get("err") is None]


def upgrade_history_fast(programdata, authority, since):
    """Same result as upgrade_history for programs whose ProgramData appears in
    many ordinary transactions: an upgrade must list the upgrade authority as
    an account, so only transactions touching both addresses are fetched.
    Misses upgrades signed by an earlier authority if it was rotated within
    the window."""
    programdata_sigs = signatures_since(programdata, since)
    authority_sigs = {entry["signature"] for entry in signatures_since(authority, since)}
    candidates = [entry for entry in programdata_sigs if entry["signature"] in authority_sigs]
    upgrades = []
    for entry in candidates:
        tx = rpc("getTransaction", [entry["signature"], {"encoding": "jsonParsed", "maxSupportedTransactionVersion": 1}])
        if tx is None:
            continue
        message = tx["transaction"]["message"]
        instructions = list(message["instructions"])
        for inner in (tx.get("meta") or {}).get("innerInstructions") or []:
            instructions.extend(inner["instructions"])
        for ix in instructions:
            parsed = ix.get("parsed") if isinstance(ix, dict) else None
            if ix.get("program") == "bpf-upgradeable-loader" and isinstance(parsed, dict) and parsed.get("type") == "upgrade":
                if parsed.get("info", {}).get("programDataAccount") == programdata:
                    outer = sorted({i.get("programId") for i in message["instructions"]})
                    upgrades.append({"signature": entry["signature"], "time": entry["blockTime"], "outer_programs": outer})
                    break
    return upgrades, len(programdata_sigs), len(candidates)


def retry_errors_fast(out_dir):
    """Re-measures failed rows with upgrade_history_fast and records the method."""
    path = f"{out_dir}/upgrade-frequency.json"
    data = json.load(open(path))
    now = data["generated_at"]
    failed = [index for index, row in enumerate(data["results"]) if "error" in row]
    for count, index in enumerate(failed):
        old = data["results"][index]
        row = {"program": old["program"], "label": old["label"]}
        try:
            row.update(program_state(old["program"]))
            if row.get("upgradeable") and row.get("authority"):
                upgrades, scanned, candidates = upgrade_history_fast(row["programdata"], row["authority"], now - 180 * DAY)
                row.update(
                    method="intersection with current upgrade authority",
                    signatures_scanned=scanned,
                    candidates_fetched=candidates,
                    upgrades_180d=len(upgrades),
                    upgrades_90d=sum(1 for u in upgrades if u["time"] >= now - 90 * DAY),
                    upgrades_30d=sum(1 for u in upgrades if u["time"] >= now - 30 * DAY),
                    upgrade_events=upgrades,
                )
        except Exception as error:
            row["error"] = str(error)
        data["results"][index] = row
        report(row, f"retry-fast {count + 1}/{len(failed)}")
        json.dump(data, open(path, "w"), indent=1)


def main():
    if sys.argv[1] == "--retry-fast":
        return retry_errors_fast(sys.argv[2])
    if sys.argv[1] == "--retry":
        return retry_errors(sys.argv[2])
    labels_path, out_dir = sys.argv[1], sys.argv[2]
    labels = json.load(open(labels_path))
    programs = {program: f"{label} (Jupiter-routed)" for program, label in labels.items()}
    for program, label in CURATED.items():
        programs[program] = label if program not in programs else programs[program] + " + curated"
    now = int(time.time())
    results = []
    for index, (program, label) in enumerate(sorted(programs.items(), key=lambda item: item[1].lower())):
        row = measure(program, label, now)
        results.append(row)
        report(row, f"{index + 1}/{len(programs)}")
    json.dump({"generated_at": now, "rpc": RPC, "results": results}, open(f"{out_dir}/upgrade-frequency.json", "w"), indent=1)


def measure(program, label, now):
    """One program's loader state and upgrade counts, windows ending at `now`."""
    row = {"program": program, "label": label}
    try:
        row.update(program_state(program))
        if row.get("upgradeable") is not None and row.get("programdata"):
            upgrades, scanned = upgrade_history(row["programdata"], now - 180 * DAY)
            row["signatures_scanned"] = scanned
            row["upgrades_180d"] = len(upgrades)
            row["upgrades_90d"] = sum(1 for u in upgrades if u["time"] >= now - 90 * DAY)
            row["upgrades_30d"] = sum(1 for u in upgrades if u["time"] >= now - 30 * DAY)
            row["upgrade_events"] = upgrades
    except Exception as error:  # keep going; record the failure
        row["error"] = str(error)
    return row


def report(row, position):
    print(f"[{position}] {row['label']}: upgradeable={row.get('upgradeable')} "
          f"90d={row.get('upgrades_90d')} 180d={row.get('upgrades_180d')} auth={row.get('authority_kind')} "
          f"{'ERROR ' + row['error'] if 'error' in row else ''}", flush=True)


def retry_errors(out_dir):
    """Re-measures rows that failed (for example on a network error), keeping
    the original run's time windows, and rewrites the results file."""
    path = f"{out_dir}/upgrade-frequency.json"
    data = json.load(open(path))
    failed = [index for index, row in enumerate(data["results"]) if "error" in row]
    for count, index in enumerate(failed):
        row = data["results"][index]
        data["results"][index] = measure(row["program"], row["label"], data["generated_at"])
        report(data["results"][index], f"retry {count + 1}/{len(failed)}")
    json.dump(data, open(path, "w"), indent=1)


if __name__ == "__main__":
    main()
