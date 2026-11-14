#!/usr/bin/env python3
"""Take upstream's extraction of the renderer's census into gfx/census.rs (83703c28b2f).

Upstream moved nine `impl Renderer` methods out of horizon-client-render's gfx.rs into
gfx/census.rs and declared `mod census;`. An earlier sync took the file without the rest. This
script does both halves with the lexical helpers of take_upstream_game_moves.py: it writes
upstream's gfx/census.rs, declares the module where upstream does (after `pub mod capture;`) and
deletes the nine methods from gfx.rs. Fork edits to the moved methods follow in separate commits.

    python3 tools/sync/take_upstream_census_move.py --upstream <sha>
"""
import argparse

import take_upstream_game_moves as moves

GFX = "crates/horizon-client-render/src/gfx.rs"
CENSUS = "crates/horizon-client-render/src/gfx/census.rs"
METHODS = {
    "resident_sections",
    "lod_quads",
    "lod_levels",
    "packed_layout",
    "vertex_bytes_split",
    "section_vertex_bytes",
    "gpu_census",
    "gpu_memory_by_label",
    "cpu_census",
}
DECLARE_AFTER = "pub mod capture;\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--upstream", required=True)
    args = parser.parse_args()
    root = moves.ROOT
    (root / CENSUS).write_text(moves.git("show", f"{args.upstream}:{CENSUS}"))
    gfx = root / GFX
    text = gfx.read_text()
    spans = [
        (it["start"], it["end"])
        for it in moves.items(text, depth_limit=1)
        if it["kind"] == "fn" and it["name"] in METHODS and it["parent"] and it["parent"]["name"] == "Renderer"
    ]
    if len(spans) != len(METHODS):
        raise SystemExit(f"found {len(spans)} of the {len(METHODS)} census methods in gfx.rs")
    text = moves.delete_spans(text, spans)
    if "mod census;\n" not in text:
        if text.count(DECLARE_AFTER) != 1:
            raise SystemExit("no single `pub mod capture;` to declare census after")
        text = text.replace(DECLARE_AFTER, DECLARE_AFTER + "mod census;\n")
    gfx.write_text(text)


if __name__ == "__main__":
    main()
