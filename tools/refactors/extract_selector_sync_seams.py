#!/usr/bin/env python3
"""Move the selector carry's existing send helper and one remote helper verbatim."""
from pathlib import Path


def move(source: str, first: str, after: str, destination: str, header: str, footer: str, declaration: str) -> None:
    path = Path(source)
    text = path.read_text()
    start = text.index(first)
    end = text.index(after, start)
    block = text[start:end]
    Path(destination).write_text(header + block + footer)
    text = text[:start] + text[end:]
    # Declarations are beside the existing child modules, not at a generated export list.
    at = text.index("\nmod ") + 1
    text = text[:at] + declaration + text[at:]
    path.write_text(text)


move("crates/horizon-client/src/lan/hub.rs",
     "    /// Publish the frozen server token inventory", "    /// The command tree a player",
     "crates/horizon-client/src/lan/hub/selector_sync.rs", "use super::*;\n\nimpl Hub {\n", "}\n",
     "mod selector_sync;\n")
move("crates/horizon-client/src/game/network/remote/mod.rs",
     "/// The hurt sound of a `ClientboundDamageEventPacket`", "/// The live entity a bound sound names",
     "crates/horizon-client/src/game/network/remote/selector_metadata.rs", "use super::*;\n\n", "",
     "mod selector_metadata;\nuse selector_metadata::hurt_sound_for;\n")
