#!/usr/bin/env python3
"""Move resource-scope behavior tests to the platform that owns scopes (#2587)."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VOCAB = ROOT / "crates/horizon-mod-vocab/src/wire"
API = ROOT / "crates/horizon-mod-api/tests"


def take_test(path, name):
    source = path.read_text()
    at = source.index(f"fn {name}()")
    start = source.rfind("#[test]", 0, at)
    # The tests moved here contain balanced Rust blocks, including closures.
    brace = source.index("{", at)
    depth = 1
    end = brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    result = source[start:end]
    path.write_text(source[:start] + source[end:])
    return result


def main():
    source = VOCAB / "wire_handlers.rs"
    text = source.read_text()
    start = text.index("#[cfg(test)]\nmod tests {")
    tests = text[start:]
    source.write_text(text[:start])
    tests = tests.replace("use super::*;", "use mod_api::*;\n    use mod_api::wire::*;")
    tests = tests.replace("crate::wire::", "mod_api::wire::")
    (API / "wire_handler_scopes.rs").write_text(tests)

    test = take_test(VOCAB / "mod.rs", "resource_context_is_scoped_and_encoding_keeps_first_error")
    (API / "wire_context.rs").write_text("use mod_api::wire as wire;\n" + test.replace("super::", "wire::") + "\n")
    test = take_test(VOCAB / "wire_layout_tests.rs", "erased_provider_record_uses_active_resource_scope")
    helper = (VOCAB / "wire_layout_tests.rs").read_text()
    start = helper.index("fn fields<")
    end = helper.index("\nfn id", start)
    decode_start = helper.index("fn decode<")
    decode_end = helper.index("\n#[test]", decode_start)
    helpers = helper[start:end] + "\n" + helper[decode_start:decode_end]
    (API / "wire_provider_layout.rs").write_text(
        "use mod_api::*;\nuse mod_api::wire::*;\n" + helpers + "\n"
        + test.replace("crate::wire::", "mod_api::wire::") + "\n"
    )


if __name__ == "__main__":
    main()
