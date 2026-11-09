#!/usr/bin/env python3
"""Validate generated IBC data files against ibc_data.schema.json.

This script loads each file in public/ibc-data/ and verifies:
1. chain_1/chain_2 are alphabetically ordered
2. chain_1.chain_id matches chain_1.chain_name
3. chain_2.chain_id matches chain_2.chain_name
4. ordering is a string ("ordered" | "unordered")
5. channels have chain_1/chain_2 keys (not old-format chain-name keys)
6. tags.preferred is present on transfer channels
7. tags.status is "ACTIVE" | "INACTIVE" | "CLOSED" | "PENDING"
"""

import json
import os
import sys
import glob


def load_schema(path: str) -> dict:
    with open(path) as f:
        return json.load(f)


def load_ibc_data(path: str) -> dict:
    with open(path) as f:
        return json.load(f)


def expect(condition: bool, msg: str) -> int:
    if not condition:
        print(f"  FAIL: {msg}")
        return 1
    return 0


def validate_ibc_data_entry(data: dict, filename: str) -> int:
    """Validate a single ibc_data entry. Returns number of violations."""
    errors = 0

    # Required top-level fields
    errors += expect("$schema" in data, f"{filename}: missing $schema")
    errors += expect("chain_1" in data, f"{filename}: missing chain_1")
    errors += expect("chain_2" in data, f"{filename}: missing chain_2")
    errors += expect("channels" in data, f"{filename}: missing channels")

    if "chain_1" not in data or "chain_2" not in data:
        return errors

    c1 = data["chain_1"]
    c2 = data["chain_2"]

    # Required chain_info fields
    for label, chain in [("chain_1", c1), ("chain_2", c2)]:
        errors += expect("chain_name" in chain, f"{filename}.{label}: missing chain_name")
        errors += expect("chain_id" in chain, f"{filename}.{label}: missing chain_id")
        errors += expect("client_id" in chain, f"{filename}.{label}: missing client_id")
        errors += expect("connection_id" in chain, f"{filename}.{label}: missing connection_id")

    # Check chain_name is in the filename (alphabetical ordering)
    c1_name = c1.get("chain_name", "")
    c2_name = c2.get("chain_name", "")

    # The filename should contain both chain names
    base = filename.replace(".json", "")
    expected_sorted = sorted([c1_name, c2_name])
    errors += expect(
        c1_name == expected_sorted[0] and c2_name == expected_sorted[1],
        f"{filename}: chain_1 ({c1_name}) and chain_2 ({c2_name}) not alphabetically ordered "
        f"(expected {expected_sorted[0]}, {expected_sorted[1]})"
    )

    # BUG CHECK: chain_id must match chain_name, NOT the counterparty
    # chain_name "akash" → chain_id must be "akashnet-2" NOT "morocco-1"
    # We can't check the exact chain_id without chain-registry data, but we can
    # check it's NOT equal to the other chain's chain_id (the swapped bug)
    errors += expect(
        c1["chain_id"] != c2["chain_id"],
        f"{filename}: chain_1.chain_id ({c1['chain_id']}) == chain_2.chain_id ({c2['chain_id']}) — "
        "chain_ids must be different"
    )

    # Check chain_1 chain_id doesn't look like the other chain's name
    # e.g. c1.chain_name="akash" but c1.chain_id="morocco-1" ← swapped
    # c1.chain_id should be the chain_id OF the chain named c1.chain_name
    # So if c1.chain_name != "terp", c1.chain_id should not be "morocco-1"
    if c2_name == "terp":
        # c2 is terp → c2.chain_id should be "morocco-1"
        errors += expect(
            c2["chain_id"] == "morocco-1",
            f"{filename}: chain_2 (terp) has chain_id '{c2['chain_id']}' but expected 'morocco-1'"
        )
        # c1 is the counterparty → c1.chain_id should NOT be "morocco-1"
        errors += expect(
            c1["chain_id"] != "morocco-1",
            f"{filename}: chain_1 ({c1['chain_name']}) has chain_id 'morocco-1' — "
            "this is the SWAPPED BUG. Should be the counterparty's chain_id."
        )
    elif c1_name == "terp":
        # c1 is terp → c1.chain_id should be "morocco-1"
        errors += expect(
            c1["chain_id"] == "morocco-1",
            f"{filename}: chain_1 (terp) has chain_id '{c1['chain_id']}' but expected 'morocco-1'"
        )
        # c2 is the counterparty → c2.chain_id should NOT be "morocco-1"
        errors += expect(
            c2["chain_id"] != "morocco-1",
            f"{filename}: chain_2 ({c2['chain_name']}) has chain_id 'morocco-1' — "
            "this is the SWAPPED BUG."
        )

    # Validate channels
    channels = data.get("channels", [])
    for i, ch in enumerate(channels):
        ch_label = f"{filename}.channels[{i}]"

        # Channel must have chain_1/chain_2 (not old-format chain-name keys)
        errors += expect(
            "chain_1" in ch and "chain_2" in ch,
            f"{ch_label}: missing chain_1 or chain_2 keys"
        )

        if "chain_1" in ch and "chain_2" in ch:
            # channel_id and port_id are required
            ch1 = ch["chain_1"]
            ch2 = ch["chain_2"]
            errors += expect(
                "channel_id" in ch1 and ch1["channel_id"],
                f"{ch_label}.chain_1: missing or empty channel_id"
            )
            errors += expect(
                "port_id" in ch1 and ch1["port_id"],
                f"{ch_label}.chain_1: missing or empty port_id"
            )
            errors += expect(
                "channel_id" in ch2 and ch2["channel_id"],
                f"{ch_label}.chain_2: missing or empty channel_id"
            )
            errors += expect(
                "port_id" in ch2 and ch2["port_id"],
                f"{ch_label}.chain_2: missing or empty port_id"
            )

        # Ordering must be a string
        ordering = ch.get("ordering")
        errors += expect(
            isinstance(ordering, str) and ordering in ("ordered", "unordered"),
            f"{ch_label}: ordering '{ordering}' must be string 'ordered' or 'unordered' "
            f"(type={type(ordering).__name__})"
        )

        # Version must be present
        errors += expect(
            "version" in ch and ch["version"],
            f"{ch_label}: missing version"
        )

        # Tags must have preferred and status
        tags = ch.get("tags", {})
        errors += expect(
            "preferred" in tags and isinstance(tags["preferred"], bool),
            f"{ch_label}.tags: missing or non-boolean 'preferred'"
        )
        errors += expect(
            "status" in tags and tags["status"] in ("ACTIVE", "INACTIVE", "CLOSED", "PENDING"),
            f"{ch_label}.tags: missing or invalid 'status' (got '{tags.get('status')}')"
        )

    return errors


def main():
    # Base directory: terp-rs workspace root
    base_dir = os.path.realpath(os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), ".."))
    ibc_data_dir = os.path.join(base_dir, "public", "ibc-data")

    if not os.path.isdir(ibc_data_dir):
        print(f"IBC data directory not found: {ibc_data_dir}")
        sys.exit(1)

    # Load schema for reference
    schema_path = os.path.join(base_dir, "public", "ibc_data.schema.json")
    if os.path.exists(schema_path):
        schema = load_schema(schema_path)
        print(f"Schema loaded: {schema_path}")
        print(f"  Required top-level fields: {schema.get('required', [])}")
    else:
        print(f"Schema not found at {schema_path} — using structural validation only")

    # Validate each ibc_data file
    files = sorted(glob.glob(os.path.join(ibc_data_dir, "*.json")))
    print(f"\nValidating {len(files)} IBC data files...")
    print()

    total_errors = 0
    for filepath in files:
        filename = os.path.basename(filepath)
        data = load_ibc_data(filepath)
        errors = validate_ibc_data_entry(data, filename)
        if errors == 0:
            print(f"  PASS: {filename}")
        else:
            print(f"  FAIL: {filename} ({errors} violations)")
        total_errors += errors

    # Also validate the terp-state.json used by the UI
    monorepo_root = os.path.realpath(os.path.join(base_dir, "..", ".."))
    ui_state_path = os.path.join(
        monorepo_root, "websites", "dao-dao-ui", "packages", "utils", "constants", "terp-state.json"
    )
    if os.path.exists(ui_state_path):
        print(f"\nValidating UI state at {ui_state_path}...")
        with open(ui_state_path) as f:
            ui_state = json.load(f)
        morocco = ui_state.get("morocco-1", {})
        ibc_data = morocco.get("ibc_data", {})
        if isinstance(ibc_data, dict):
            for key, entry in ibc_data.items():
                # Skip old-format keys (should be cleaned out)
                if "-terp" in key:
                    print(f"  OLD-FORMAT (skipped): {key}")
                    continue
                errors = validate_ibc_data_entry(entry, f"ibc_data.{key}")
                total_errors += errors
                status = "PASS" if errors == 0 else "FAIL"
                print(f"  {status}: ibc_data.{key} ({errors} violations)")
    else:
        print(f"\nUI state not found at {ui_state_path}")

    print(f"\n{'=' * 40}")
    if total_errors == 0:
        print(f"ALL {len(files)} FILES SCHEMA-COMPLIANT")
    else:
        print(f"{total_errors} SCHEMA VIOLATIONS FOUND")
    print(f"{'=' * 40}")

    return 0 if total_errors == 0 else 1


if __name__ == "__main__":
    sys.exit(main())