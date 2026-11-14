#!/usr/bin/env python3
"""Make the shared holder-set universe explicitly fallible across its implementations."""
from pathlib import Path
import re

paths = [
    "crates/horizon-world/src/holder_set.rs",
    "crates/horizon-world/src/worldgen/block_feature.rs",
    "crates/horizon-world/src/worldgen/biome_predicate.rs",
    "crates/horizon-world/src/worldgen/generator_structures.rs",
    "crates/horizon-world/src/worldgen/biome_source/codec.rs",
    "crates/horizon-world/src/worldgen/fluid_predicate.rs",
    "crates/horizon-client/src/mod_content_biomes.rs",
]
for name in paths:
    path = Path(name)
    source = path.read_text()
    source = source.replace("fn all(&self) -> Vec<Self::Id>;", "fn all(&self) -> Result<Vec<Self::Id>, String>;")
    def convert(match):
        indent, item, body = match.groups()
        if "self.ids.map" in body:
            body = 'self.ids.map(|ids| ids()).ok_or_else(|| format!("{} universe is unavailable", self.what))'
        else:
            body = "Ok(" + body.strip() + ")"
        result = "std::result::Result" if "mod_content_biomes" in name else "Result"
        return f"{indent}fn all(&self) -> {result}<Vec<{item}>, String> {{\n{indent}    {body}\n{indent}}}"
    source = re.sub(r"(?m)^([ \t]*)fn all\(&self\) -> Vec<([^\n]+)> \{\n(.*?)^\1\}", convert, source, flags=re.S)
    source = source.replace("unique(lookup.all())", "unique(lookup.all()?)")
    path.write_text(source)
