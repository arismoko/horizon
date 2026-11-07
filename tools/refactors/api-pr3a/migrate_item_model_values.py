#!/usr/bin/env python3
"""Seed the four item-model hook target and guarded evaluation shape changes.

Compiler-guided companion edits add the shared StackView conversion, catalog docs,
owned-stack field reads in tests and the book's migration table.
"""
from pathlib import Path

names = ("item_model_property", "item_model_range_property", "item_model_select_property", "item_tint_source")
for name in names:
    path = Path(f"crates/horizon-client/src/game/native_hooks/{name}.rs")
    text = path.read_text().replace("stack: &StackView,", "stack: &mod_api::ItemStack,\n        bundle_selected: i32,")
    text = text.replace("property, stack, value", "property, stack, bundle_selected, value")
    text = text.replace("source, config, stack, color", "source, config, stack, bundle_selected, color")
    path.write_text(text)

path = Path("crates/horizon-client/src/game/interface/item_draw.rs")
text = path.read_text().replace("use super::{Dim, Game};", "use super::{Dim, Game};\nuse super::api_values::TryIntoApi;")
for name, default in zip(names, ("false", "0.0", "None", "-1")):
    start = text.index(f"    fn {name}(")
    end = text.index("\n    fn ", start + 1)
    method = text[start:end]
    declaration = "let mut color = u32::MAX;" if name == "item_tint_source" else f"let mut value = {default};"
    method = method.replace(declaration, f"let Ok(snapshot) = stack.try_into_api() else {{ return {default}; }};\n        {declaration}")
    method = method.replace("property, stack, &mut value", "property, &snapshot, stack.bundle_selected, &mut value")
    method = method.replace("source, config, stack, &mut color", "source, config, &snapshot, stack.bundle_selected, &mut color")
    text = text[:start] + method + text[end:]
text = text.replace("event.stack.count >", "event.stack.count() >").replace("match event.stack.count {", "match event.stack.count() {")
path.write_text(text)
