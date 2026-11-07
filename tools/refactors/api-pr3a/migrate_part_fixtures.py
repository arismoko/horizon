#!/usr/bin/env python3
"""Mechanical reads in the two owned stack fixture files; mutations are hand-written."""
from pathlib import Path
import re
root=Path(__file__).resolve().parents[3]
p=root/'crates/horizon-mod-api/src/parts/items.rs'
s=p.read_text()
s=s.replace('same_item_same_components','is_stackable_with')
s=re.sub(r'\.(damage|count)\b(?!\s*\(|\s*(?:[+\-]?=))',r'.\1()',s)
s=re.sub(r'(\w+)\.item\(\) == "([^"]+)"',r'\1.item() == fixture_item_key("\2")',s)
s=s.replace('resource.item() == item','resource.item() == fixture_item_key(item)')
s=s.replace('r.item() != "test:bag"','r.item() != fixture_item_key("test:bag")').replace('resource.item() != "minecraft:stick"','resource.item() != fixture_item_key("minecraft:stick")')
s=s.replace('&*s.item == item','s.item() == fixture_item_key(item)')
s=s.replace('match &*s.item {','match crate::registry_keys::item_id(s.item()).unwrap() {')
s=s.replace('resource.to_stack(1)','resource.to_stack(1).unwrap()').replace('r.to_stack(1).damage()', 'r.to_stack(1).unwrap().damage()')
s=s.replace('let stack = ItemStack::new("minecraft:stick", 1);','let stack = stick();')
p.write_text(s)
