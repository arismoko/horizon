#!/usr/bin/env python3
"""Move the tooltip guard attribution test to its platform owner (#2587)."""
from pathlib import Path
from move_resource_scope_tests import take_test

ROOT = Path(__file__).resolve().parents[2]
source = ROOT / "crates/horizon-mod-vocab/src/parts/items.rs"
test = take_test(source, "tooltip_appender_restores_owner_even_on_panic")
target = ROOT / "crates/horizon-mod-api/tests/tooltip_attribution.rs"
target.write_text("#![cfg(feature = \"testing\")]\nuse mod_api::*;\n" + test + "\n")
