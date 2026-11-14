#!/usr/bin/env python3
"""Move loot resource reads verbatim before adding load-owned holder-set binding."""
from pathlib import Path

source = Path("crates/horizon-world/src/loot.rs")
text = source.read_text()
start = text.index("fn load_kind(")
end = text.index("fn cost(", start)
body = text[start:end]
Path("crates/horizon-world/src/loot/load_resources.rs").write_text("use super::*;\n\n" + body)
source.write_text(text[:start] + "#[path = \"loot/load_resources.rs\"]\nmod load_resources;\nuse load_resources::{hash_files, load_dir, load_kind};\n\n" + text[end:])
