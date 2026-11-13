#!/usr/bin/env python3
"""Move credited remote ingress staging into a cohesive child module."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
remote = root / "crates/horizon-client/src/game/network/remote"
path = remote / "mod.rs"
source = path.read_text()
start = source.index("        // Chunk packets start decoding now; the frame that reaches one in")
end = source.index("        let overdue = r", start)
body = source[start:end].replace("r.session", "self.session").replace("r.decoder", "self.decoder").replace("r.chunk_format", "self.chunk_format")
source = source[:start] + "        r.receive_credited_backlog(&mut events);\n" + source[end:]
source = source.replace("mod recipe_properties;", "mod recipe_properties;\nmod raw_ingress;")
path.write_text(source)
(remote / "raw_ingress.rs").write_text(
    "//! Receiving-provider credit follows ordered events and decoder placeholders.\n"
    "use super::*;\n\n"
    "impl Remote {\n"
    "    pub(super) fn receive_credited_backlog(&mut self, events: &mut VecDeque<Received>) {\n"
    + body
    + "    }\n}\n"
)
