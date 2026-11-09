#!/usr/bin/env python3
"""One-time migration: fix all generated IBC data files to be schema-compliant.

This script reads the existing buggy files in public/ibc-data/ and the
terp-state.json at dao-dao-ui, then rewrites them with:

1. chain_1/chain_2 alphabetically ordered with correct chain_ids
2. ordering as string enum ("ordered" | "unordered")
3. tags.preferred boolean on all channels
4. Clean channel chain_1/chain_2 structure (no old-format chain-name keys)

After this migration, running `cargo run --bin ibc` will produce correct
files from the live chain. This script exists only to fix what's already
generated.
"""

import json
import os
import glob
import sys


def fix_channel(ch: dict, chain_1_name: str, chain_2_name: str,
                has_preferred_transfer: list) -> dict:
    """Rebuild a channel entry in schema-compliant format.

    The raw gRPC data has ordering as an integer (1=UNORDERED, 2=ORDERED).
    Channel structure may use chain-name keys instead of chain_1/chain_2.
    """
    # Determine if this is a transfer channel
    port_1 = ch.get("chain_1", {}).get("port_id", "") or ""
    port_2 = ch.get("chain_2", {}).get("port_id", "") or ""

    # Handle old format where channel keys are chain names instead of chain_1/chain_2
    if "chain_1" not in ch or "chain_2" not in ch:
        # Old format: keys might be chain_name pairs like {"akash": {...}, "terp": {...}}
        ch1_info = ch.get(chain_1_name, {}) or {}
        ch2_info = ch.get(chain_2_name, {}) or {}
        port_1 = ch1_info.get("port_id", "") or ""
        port_2 = ch2_info.get("port_id", "") or ""
        chan_id_1 = ch1_info.get("channel_id", "") or ""
        chan_id_2 = ch2_info.get("channel_id", "") or ""
    else:
        chan_id_1 = ch.get("chain_1", {}).get("channel_id", "") or ""
        chan_id_2 = ch.get("chain_2", {}).get("channel_id", "") or ""
        port_1 = ch.get("chain_1", {}).get("port_id", "") or ""
        port_2 = ch.get("chain_2", {}).get("port_id", "") or ""

    # If still empty, try alternate versions
    if not chan_id_1:
        chan_id_1 = ch.get("chain_1", {}).get("channelId", "") or ""
    if not chan_id_2:
        chan_id_2 = ch.get("chain_2", {}).get("channelId", "") or ""
    if not port_1:
        port_1 = ch.get("chain_1", {}).get("portId", "") or "transfer"
    if not port_2:
        port_2 = ch.get("chain_2", {}).get("portId", "") or "transfer"

    # Convert ordering to string
    ordering = ch.get("ordering", 1)
    if isinstance(ordering, int):
        ordering_str = "unordered" if ordering == 1 else "ordered"
    elif isinstance(ordering, str):
        ordering_str = ordering
    else:
        ordering_str = "unordered"

    # Tags: add preferred if missing
    tags = dict(ch.get("tags", {}))
    is_transfer = (port_1 == "transfer" and port_2 == "transfer")
    if "preferred" not in tags:
        if is_transfer and not has_preferred_transfer[0]:
            tags["preferred"] = True
            has_preferred_transfer[0] = True
        elif is_transfer:
            tags["preferred"] = False
        else:
            tags["preferred"] = True
    if "status" not in tags:
        tags["status"] = "ACTIVE"

    return {
        "chain_1": {
            "channel_id": chan_id_1,
            "port_id": port_1,
        },
        "chain_2": {
            "channel_id": chan_id_2,
            "port_id": port_2,
        },
        "ordering": ordering_str,
        "version": ch.get("version", "ics20-1"),
        "tags": tags,
    }


def fix_ibc_data_entry(entry: dict, filename: str) -> dict:
    """Fix a single ibc_data entry to be schema-compliant."""
    chain_1_data = entry.get("chain_1", {})
    chain_2_data = entry.get("chain_2", {})

    if not chain_1_data or not chain_2_data:
        # Old format: keys are chain names
        # Extract chain names from keys
        keys = [k for k in entry.keys() if k not in ("$schema", "channels")]
        if len(keys) >= 2:
            # Find terp and the counterparty
            if "terp" in keys:
                cp_key = [k for k in keys if k != "terp"][0]
                chain_1_data = entry.get(cp_key, {})
                chain_2_data = entry.get("terp", {})
            else:
                chain_1_data = entry.get(keys[0], {})
                chain_2_data = entry.get(keys[1], {})
        # If chain_1/chain_2 are present but swapped chain_ids, fix that
    elif chain_1_data.get("chain_name") != "terp" and chain_1_data.get("chain_id") == "morocco-1":
        # SWAPPED BUG: chain_1 has terp's chain_id but non-terp chain_name
        # Fix: swap chain_id values. chain_1 (counterparty) should have counterparty's
        # chain_id which was sitting in chain_2. chain_2 (terp) should be "morocco-1".
        c1_name = chain_1_data.get("chain_name", "")
        c1_cid = chain_1_data.get("client_id", "")
        c1_conn = chain_1_data.get("connection_id", "")
        c2_name = chain_2_data.get("chain_name", "")
        c2_cid = chain_2_data.get("client_id", "")
        c2_conn = chain_2_data.get("connection_id", "")
        # The real counterparty chain_id is in chain_2.chain_id (was swapped)
        c1_new_id = chain_2_data.get("chain_id", "unknown")
        c2_new_id = "morocco-1"
        # Also fix channels
        has_pref = [False]
        fixed_channels = []
        for ch in entry.get("channels", []):
            # Ensure channel has chain_1/chain_2 keys before passing to fix_channel
            if "chain_1" not in ch or "chain_2" not in ch:
                # Old format — try to extract from chain-name keys
                fixed_ch = fix_channel(
                    {"chain_1": ch.get(c1_name, {}) or {},
                     "chain_2": ch.get(c2_name, {}) or {},
                     "ordering": ch.get("ordering", 1),
                     "version": ch.get("version", "ics20-1"),
                     "tags": ch.get("tags", {})},
                    c1_name, c2_name, has_pref
                )
            else:
                fixed_ch = fix_channel(ch, c1_name, c2_name, has_pref)
            fixed_channels.append(fixed_ch)
        return {
            "$schema": "../ibc_data.schema.json",
            "chain_1": {
                "chain_name": c1_name,
                "chain_id": c1_new_id,
                "client_id": c1_cid,
                "connection_id": c1_conn,
            },
            "chain_2": {
                "chain_name": c2_name,
                "chain_id": c2_new_id,
                "client_id": c2_cid,
                "connection_id": c2_conn,
            },
            "channels": fixed_channels,
        }

    # Get chain names for alphabetical ordering
    c1_name = chain_1_data.get("chain_name", "")
    c2_name = chain_2_data.get("chain_name", "")

    # If chain_1 and chain_2 exist but ordering is violated, swap them
    if sorted([c1_name, c2_name]) != [c1_name, c2_name]:
        chain_1_data, chain_2_data = chain_2_data, chain_1_data
        c1_name, c2_name = c2_name, c1_name

    # Fix channels
    has_pref = [False]
    fixed_channels = [
        fix_channel(ch, c1_name, c2_name, has_pref)
        for ch in entry.get("channels", [])
    ]

    return {
        "$schema": "../ibc_data.schema.json",
        "chain_1": chain_1_data,
        "chain_2": chain_2_data,
        "channels": fixed_channels,
    }


def main():
    # Paths relative to this script location: crates/terp-rs/tests/scripts/
    script_dir = os.path.dirname(os.path.abspath(__file__))
    repo_root = os.path.realpath(os.path.join(script_dir, "..", ".."))  # up from tests/scripts/ to terp-rs/
    ibc_data_dir = os.path.join(repo_root, "public", "ibc-data")

    # Fix public/ibc-data/*.json
    files = sorted(glob.glob(os.path.join(ibc_data_dir, "*.json")))
    print(f"Fixing {len(files)} IBC data files in {ibc_data_dir}...")
    for filepath in files:
        filename = os.path.basename(filepath)
        with open(filepath) as f:
            data = json.load(f)
        fixed = fix_ibc_data_entry(data, filename)
        with open(filepath, "w") as f:
            json.dump(fixed, f, indent=2)
            f.write("\n")
        print(f"  Fixed {filename}")

    # Fix terp-state.json
    monorepo_root = os.path.realpath(os.path.join(repo_root, "..", ".."))
    ui_state_path = os.path.join(
        monorepo_root, "websites", "dao-dao-ui", "packages", "utils", "constants", "terp-state.json"
    )
    if os.path.exists(ui_state_path):
        print(f"\nFixing UI state at {ui_state_path}...")
        with open(ui_state_path) as f:
            state = json.load(f)
        morocco = state.get("morocco-1", {})
        if "ibc_data" in morocco:
            ibc_data = morocco["ibc_data"]
            fixed_ibc = {}
            for key, entry in ibc_data.items():
                if "-terp" in key:
                    print(f"  Removing old-format entry: {key}")
                    continue  # Skip old-format entries (clean them out)
                fixed_ibc[key] = fix_ibc_data_entry(entry, key)
                print(f"  Fixed ibc_data.{key}")
            morocco["ibc_data"] = fixed_ibc
        with open(ui_state_path, "w") as f:
            json.dump(state, f, indent=2)
            f.write("\n")
        print(f"  UI state updated")

    print("\nMigration complete.")

    # Re-run validation
    print("\nRe-running validation...\n")
    import subprocess
    result = subprocess.run(
        [sys.executable, os.path.join(script_dir, "validate_ibc_schema.py")],
        capture_output=True, text=True
    )
    print(result.stdout)
    if result.returncode != 0:
        print("VALIDATION FAILED — some files still violate the schema")
        print(result.stderr if result.stderr else "")
    else:
        print("ALL FILES SCHEMA-COMPLIANT")


if __name__ == "__main__":
    main()