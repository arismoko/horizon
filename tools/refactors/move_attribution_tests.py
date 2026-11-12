#!/usr/bin/env python3
"""Move attribution tests to the platform instrumentation owner (#2587)."""
from pathlib import Path
from move_resource_scope_tests import take_test

ROOT = Path(__file__).resolve().parents[2]

for side, folder in (("server", "world"), ("client", "client")):
    source = ROOT / f"crates/horizon-mod-vocab/src/{folder}/{side}_reload_tests.rs"
    test = take_test(source, "add_listener_attributes_its_calls_to_the_registering_mod")
    imports = f"use mod_api::*;\nuse mod_api::__private::game::{side.title()}ReloadListenersOrder;\nuse std::sync::Arc;\n"
    helpers = """
fn id(value: &str) -> Identifier { Identifier::parse(value).unwrap() }
fn files() -> FileToIdConverter { FileToIdConverter::json("machines").unwrap() }
"""
    test = test.replace("crate::content::crash::", "mod_api::content::crash::")
    target = ROOT / f"crates/horizon-mod-api/tests/{side}_reload_attribution.rs"
    target.write_text(imports + helpers + "\n" + test + "\n")
