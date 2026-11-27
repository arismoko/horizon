#!/usr/bin/env python3
"""Mechanical debt 2r migration: rename mod-API Transaction to TransactionContext.

Usage:
    python3 tools/debt_2r_transaction_context.py
        Apply the word-boundary rename Transaction -> TransactionContext across the
        reviewed Rust files, and rewrite `run_transaction(host, ` /
        `mod_api::run_transaction(host, ` calls to `host.transaction(` in those
        files (excluding `item/transaction.rs` and `exports/transaction.rs`).

    python3 tools/debt_2r_transaction_context.py --check
    python3 tools/debt_2r_transaction_context.py --inventory
        Report remaining occurrences of legacy transaction symbols across `crates/`.
        `--check` exits non-zero when matches remain; `--inventory` exits 0.
"""

from __future__ import annotations

import argparse
from collections import Counter
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]

REVIEWED_FILES: tuple[str, ...] = (
    # sunrise-mod-vocab
    "crates/sunrise-mod-vocab/src/block/block_entity_view.rs",
    "crates/sunrise-mod-vocab/src/block/bundle_items.rs",
    "crates/sunrise-mod-vocab/src/block/fluid_capability.rs",
    "crates/sunrise-mod-vocab/src/block/item_place.rs",
    "crates/sunrise-mod-vocab/src/content/attached_storage.rs",
    "crates/sunrise-mod-vocab/src/content/resource.rs",
    "crates/sunrise-mod-vocab/src/content/resource_util.rs",
    "crates/sunrise-mod-vocab/src/content/resource_util_tests.rs",
    "crates/sunrise-mod-vocab/src/entity/entity_capability.rs",
    "crates/sunrise-mod-vocab/src/exports/transaction.rs",
    "crates/sunrise-mod-vocab/src/host/block_entities.rs",
    "crates/sunrise-mod-vocab/src/host/fluids_energy.rs",
    "crates/sunrise-mod-vocab/src/host/items.rs",
    "crates/sunrise-mod-vocab/src/host/mod_state.rs",
    "crates/sunrise-mod-vocab/src/item/capability.rs",
    "crates/sunrise-mod-vocab/src/item/energy_capability.rs",
    "crates/sunrise-mod-vocab/src/item/energy_capability_tests.rs",
    "crates/sunrise-mod-vocab/src/item/item_component_handlers.rs",
    "crates/sunrise-mod-vocab/src/item/item_stack.rs",
    "crates/sunrise-mod-vocab/src/item/storage.rs",
    "crates/sunrise-mod-vocab/src/item/transaction.rs",
    "crates/sunrise-mod-vocab/src/testing.rs",
    "crates/sunrise-mod-vocab/src/testing/journal.rs",
    # sunrise-mod-api
    "crates/sunrise-mod-api/src/parts/capabilities.rs",
    "crates/sunrise-mod-api/src/parts/energy.rs",
    "crates/sunrise-mod-api/src/parts/entities.rs",
    "crates/sunrise-mod-api/src/parts/fluids.rs",
    "crates/sunrise-mod-api/src/parts/items.rs",
    # sunrise-client
    "crates/sunrise-client/src/cut_layout/game/blocks/dispense_container_tests.rs",
    "crates/sunrise-gameplay/src/game/blocks/machines/native_energy.rs",
    "crates/sunrise-gameplay/src/game/blocks/machines/native_fluids.rs",
    "crates/sunrise-gameplay/src/game/blocks/machines/native_items.rs",
    "crates/sunrise-client/src/cut_layout/game/blocks/machines/storage_attempt_tests.rs",
    "crates/sunrise-gameplay/src/game/blocks/world_fluids.rs",
    "crates/sunrise-client/src/cut_layout/game/modding/native_block_entity_item_tests.rs",
    "crates/sunrise-gameplay/src/game/modding/native_block_entity_refs.rs",
    "crates/sunrise-client/src/cut_layout/game/modding/native_capability_cache_tests.rs",
    "crates/sunrise-gameplay/src/game/modding/native_item_places.rs",
    "crates/sunrise-gameplay-common/src/game/runtime/transactions.rs",
    "crates/sunrise-gameplay-common/src/game/runtime/transactions/scratch.rs",
)

# `run_transaction` definition and re-export stay for Step 2.
RUN_TX_REWRITE_SKIP: frozenset[str] = frozenset(
    {
        "crates/sunrise-mod-vocab/src/item/transaction.rs",
        "crates/sunrise-mod-vocab/src/exports/transaction.rs",
    }
)

# Excluded from inventory scans: unrelated SQLite Transaction and hook-note prose.
EXCLUDED_PREFIXES: tuple[str, ...] = ("crates/sunrise-telemetry-server/",)
EXCLUDED_FILES: frozenset[str] = frozenset(
    {
        "crates/sunrise-mod-vocab/src/hooks/notes.rs",
    }
)

# Match mod-API `Transaction` (and `mod_api::Transaction`), preserving dotted Java
# class references in comments such as `Transaction.openRoot` or `Transaction.Lifecycle`.
TRANSACTION_TYPE_RE = re.compile(r"\bTransaction\b(?!\.)")
RUN_TRANSACTION_HOST_RE = re.compile(r"\b(?:mod_api::)?run_transaction\(host,\s*")

INVENTORY_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("Transaction", re.compile(r"\bTransaction\b")),
    ("TransactionSeal", re.compile(r"\bTransactionSeal\b")),
    ("transaction_open", re.compile(r"\btransaction_open\b")),
    ("transaction_close", re.compile(r"\btransaction_close\b")),
    ("transaction_current", re.compile(r"\btransaction_current\b")),
    ("transaction_depth", re.compile(r"\btransaction_depth\b")),
    ("transaction_holds", re.compile(r"\btransaction_holds\b")),
    ("run_transaction", re.compile(r"\brun_transaction\b")),
)


def apply_migration() -> int:
    missing = [rel for rel in REVIEWED_FILES if not (ROOT / rel).is_file()]
    if missing:
        for rel in missing:
            print(f"error: missing reviewed file: {rel}", file=sys.stderr)
        return 1

    touched = 0
    total_type_renames = 0
    total_run_rewrites = 0

    for rel in REVIEWED_FILES:
        path = ROOT / rel
        original = path.read_text(encoding="utf-8")
        updated, type_count = TRANSACTION_TYPE_RE.subn("TransactionContext", original)
        run_count = 0
        if rel not in RUN_TX_REWRITE_SKIP:
            updated, run_count = RUN_TRANSACTION_HOST_RE.subn(
                "host.transaction(", updated
            )
        if updated != original:
            path.write_text(updated, encoding="utf-8")
            touched += 1
            total_type_renames += type_count
            total_run_rewrites += run_count
            print(
                f"updated {rel}: {type_count} type rename(s), "
                f"{run_count} host.transaction rewrite(s)"
            )

    print(
        f"done: {touched} file(s) updated, {total_type_renames} type rename(s), "
        f"{total_run_rewrites} host.transaction rewrite(s)"
    )
    return 0


def is_excluded(rel: str) -> bool:
    return rel in EXCLUDED_FILES or any(
        rel.startswith(prefix) for prefix in EXCLUDED_PREFIXES
    )


def run_inventory(fail_on_matches: bool) -> int:
    counts: Counter[str] = Counter()
    crates_dir = ROOT / "crates"

    for path in sorted(crates_dir.rglob("*.rs")):
        rel = path.relative_to(ROOT).as_posix()
        if is_excluded(rel):
            continue
        lines = path.read_text(encoding="utf-8").splitlines()
        for lineno, line in enumerate(lines, 1):
            for name, pattern in INVENTORY_PATTERNS:
                matches = pattern.findall(line)
                if matches:
                    counts[name] += len(matches)
                    print(f"{rel}:{lineno}: [{name}] {line.strip()}")

    print("\nSummary:")
    total = 0
    for name, _ in INVENTORY_PATTERNS:
        count = counts[name]
        total += count
        print(f"  {name}: {count}")
    print(f"  total: {total}")

    if fail_on_matches and total > 0:
        return 1
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Debt 2r mechanical rename (Transaction -> TransactionContext) and "
            "legacy transaction symbol inventory."
        )
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check",
        action="store_true",
        help="Report remaining legacy transaction symbols across crates/ and exit 1 if any remain.",
    )
    mode.add_argument(
        "--inventory",
        action="store_true",
        help="Report remaining legacy transaction symbols across crates/ and exit 0.",
    )
    args = parser.parse_args(argv)

    if args.check:
        return run_inventory(fail_on_matches=True)
    if args.inventory:
        return run_inventory(fail_on_matches=False)
    return apply_migration()


if __name__ == "__main__":
    raise SystemExit(main())
