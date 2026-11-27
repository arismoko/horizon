#!/usr/bin/env python3
"""Compare GUI sprite metadata (GuiMetadataSection.CODEC through ResourceMetadata) and the tiled copies
TiledBlitRenderState builds, bit for bit, with the pinned client."""
import argparse
import json
import os
from pathlib import Path
import random
import subprocess

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base, test_model_baking_parity as baking

NUMBERS = ['null', 'true', 'false', '0', '1', '-1', '16', '16.0', '16.7', '-0.9', '0.5', '1e1', '1E2', '1e100',
           '2147483647', '2147483648', '-2147483648', '4294967296', '4294967312', '4294967297.5',
           '9223372036854775808', '18446744073709551617', '"16"', '{}', '[]', '[16]']


def mcmeta(scaling):
    return '{"gui":{"scaling":' + scaling + '}}'


def corpus():
    def decode(name, text):
        return {"kind": "decode", "name": name, "json": text}
    # Whole files: no section, other sections, broken JSON and wrong shapes.
    for i, text in enumerate(['{}', '{"animation":{}}', '{"gui":{}}', '{"gui":{"scaling":{"type":"stretch"}}}',
                              '{"gui":{"scaling":{"type":"stretch","width":0}}}', '{"gui":null}', '{"gui":[]}',
                              '{"gui":1}', '{"gui":{"scaling":null}}', '{"gui":{"scaling":[]}}', '[]', '1', 'null',
                              '', 'invalid', '{"gui":{"scaling":{}}}', '{"gui":{"scaling":{"type":null}}}',
                              '{"gui":{"scaling":{"type":1}}}', '{"gui":{"scaling":{"type":"TILE"}}}',
                              '{"gui":{"scaling":{"type":"tiles"}}}', '{"gui":{"scaling":{"type":""}}}',
                              '{"gui":{"scaling":{"type":"tile","width":4,"height":4}},'
                              '"gui":{"scaling":{"type":"tile","width":8,"height":8}}}',
                              '{"gui":{"scaling":{"type":"tile","width":4,"width":9,"height":4}}}',
                              '{"gui":{"scaling":{"type":"tile","width":4,"height":4}}} // a note',
                              '﻿{"gui":{"scaling":{"type":"tile","width":4,"height":4}}}']):
        yield decode(f"file-{i}", text)
    # Tile sizes: every awkward number on each axis.
    for i, number in enumerate(NUMBERS):
        yield decode(f"tile-width-{i}", mcmeta('{"type":"tile","width":' + number + ',"height":16}'))
        yield decode(f"tile-height-{i}", mcmeta('{"type":"tile","width":16,"height":' + number + '}'))
    yield decode("tile-missing-width", mcmeta('{"type":"tile","height":16}'))
    yield decode("tile-missing-height", mcmeta('{"type":"tile","width":16}'))
    # Nine slices: sizes, the border as one number or four sides, stretch_inner, validate.
    for i, number in enumerate(NUMBERS):
        yield decode(f"nine-width-{i}", mcmeta('{"type":"nine_slice","width":' + number + ',"height":20,"border":3}'))
        yield decode(f"nine-border-{i}", mcmeta('{"type":"nine_slice","width":200,"height":20,"border":' + number + '}'))
        yield decode(f"nine-side-{i}", mcmeta('{"type":"nine_slice","width":200,"height":20,"border":'
                                              '{"left":' + number + ',"top":1,"right":2,"bottom":3}}'))
        yield decode(f"nine-stretch-{i}", mcmeta('{"type":"nine_slice","width":200,"height":20,"border":3,'
                                                 '"stretch_inner":' + number + '}'))
    for i, border in enumerate(['{"left":1,"top":1,"right":1}', '{"left":0,"top":0,"right":0,"bottom":0}',
                                '{"left":1,"top":2,"right":3,"bottom":4,"extra":5}',
                                '{"left":1073741824,"top":1,"right":1073741824,"bottom":1}',
                                '{"left":2147483647,"top":0,"right":2147483647,"bottom":0}',
                                '{"left":9,"top":5,"right":10,"bottom":4}', '{"left":10,"top":5,"right":10,"bottom":4}',
                                '{"left":5,"top":9,"right":5,"bottom":10}', '{"left":5,"top":10,"right":5,"bottom":10}',
                                '9', '10', '11']):
        yield decode(f"nine-edge-{i}", mcmeta('{"type":"nine_slice","width":20,"height":20,"border":' + border + '}'))
    for i, value in enumerate(['true', 'false', '"true"', 'null', '1', '0', '256', '257', '-256', '0.5']):
        yield decode(f"stretch-inner-{i}", mcmeta('{"type":"nine_slice","width":20,"height":20,"border":3,'
                                                  '"stretch_inner":' + value + '}'))

    # Tiled copies: atlas placement, the sprite's GUI size, the source region and the destination.
    def tile(name, atlas, at, contents, sprite, texture, tile_size, dst):
        return {"kind": "tile", "name": name, "atlas_w": atlas[0], "atlas_h": atlas[1], "x": at[0], "y": at[1],
                "contents_w": contents[0], "contents_h": contents[1], "sprite_w": sprite[0], "sprite_h": sprite[1],
                "texture_x": texture[0], "texture_y": texture[1], "tile_w": tile_size[0], "tile_h": tile_size[1],
                "x0": dst[0], "y0": dst[1], "w": dst[2], "h": dst[3]}
    # GuiSpriteScaling.Tile: the whole sprite at the tile's size.
    for i, (atlas, at, contents, size, dst) in enumerate([
            ((256, 256), (0, 0), (16, 16), (16, 16), (0, 0, 40, 20)),
            ((256, 256), (32, 64), (16, 16), (16, 16), (3, 7, 16, 16)),
            ((256, 256), (32, 64), (16, 16), (16, 16), (3, 7, 5, 9)),
            ((1000, 600), (37, 101), (12, 20), (6, 10), (11, 13, 37, 23)),
            ((1000, 600), (37, 101), (12, 20), (6, 10), (0, 0, 5, 4)),
            ((512, 256), (64, 32), (256, 128), (7, 3), (1, 2, 50, 31)),
            ((96, 80), (12, 40), (80, 40), (80, 40), (0, 0, 81, 41)),
            ((8, 8), (0, 0), (8, 8), (1, 1), (0, 0, 3, 2))]):
        yield tile(f"tile-{i}", atlas, at, contents, size, (0, 0), size, dst)
    # A nine slice's inner segment: part of the sprite repeated.
    for i, (texture, segment, dst) in enumerate([((3, 0), (194, 20), (3, 0, 92, 20)), ((3, 3), (194, 14), (3, 3, 400, 33)),
                                                ((0, 3), (3, 14), (0, 3, 3, 15)), ((5, 6), (7, 11), (2, 2, 30, 40))]):
        yield tile(f"segment-{i}", (512, 512), (100, 300), (400, 40), (200, 20), texture, segment, dst)
    rng = random.Random(0x5355_4E52)
    for i in range(400):
        atlas = (rng.choice([64, 256, 1000, 1024, 4096, 333]), rng.choice([64, 256, 600, 1024, 4096, 77]))
        contents = (rng.randint(1, min(atlas[0], 64)), rng.randint(1, min(atlas[1], 64)))
        at = (rng.randint(0, atlas[0] - contents[0]), rng.randint(0, atlas[1] - contents[1]))
        sprite = (rng.randint(1, 64), rng.randint(1, 64))
        texture = (rng.randint(0, sprite[0] - 1), rng.randint(0, sprite[1] - 1))
        size = (rng.randint(1, sprite[0] - texture[0]), rng.randint(1, sprite[1] - texture[1]))
        dst = (rng.randint(-8, 64), rng.randint(-8, 64), rng.randint(1, 90), rng.randint(1, 90))
        yield tile(f"random-{i}", atlas, at, contents, sprite, texture, size, dst)


def canonical(path):
    """Each output line with its keys sorted, so Gson's and serde_json's key orders compare equal."""
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    path.write_text("".join(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n" for row in rows),
                    encoding="utf-8")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=["dev", "release"], default="release")
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / "rust-gui-sprite-scaling-parity"
    build.mkdir(parents=True, exist_ok=True)
    fixtures = list(corpus())
    inputs = build / "inputs.jsonl"
    inputs.write_text("".join(json.dumps(row) + "\n" for row in fixtures), encoding="utf-8")
    classes = build / "classes"
    classes.mkdir(exist_ok=True)
    oracle = lab.ROOT / "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaGuiSpriteScalingOracle.java"
    probe = lab.ROOT / "crates/sunrise-client-render/examples/gui_sprite_scaling_probe.rs"
    tested_paths = [Path(__file__), oracle, probe, lab.ROOT / "crates/sunrise-client-render/src/ui.rs",
                    lab.ROOT / "crates/sunrise-level/src/java_text.rs",
                    lab.ROOT / "crates/sunrise-client-render/src/packs.rs", lab.ROOT / "Cargo.lock"]
    tested_sources = base.source_hashes(tested_paths)
    tested_fixtures = base.source_hashes([inputs])
    dependencies = os.pathsep.join(map(str, [jars["client"], *baking.client_libraries()]))

    def run(command, name):
        with (build / (name + ".log")).open("w", encoding="utf-8") as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)

    run(["javac", "--release", lock["minecraft"]["java_major"], "-proc:none", "-cp", dependencies, "-d", classes,
         oracle], "compile-java")
    run(["cargo", "build", "--profile", args.profile, "-p", "sunrise-client-render", "--example",
         "gui_sprite_scaling_probe"], "compile-rust")
    target = lab.cargo_target() / ("debug" if args.profile == "dev" else "release")
    run([lab.binary(target / "examples", "gui_sprite_scaling_probe"), inputs, build / "rust.jsonl"], "rust")
    run(["java", "-Xmx512M", "-cp", str(classes) + os.pathsep + dependencies,
         "dev.sunriselab.parity.VanillaGuiSpriteScalingOracle", inputs, build / "client.jsonl"], "client")
    native, rust = canonical(build / "client.jsonl"), canonical(build / "rust.jsonl")
    if [row["name"] for row in native] != [row["name"] for row in fixtures] or len(rust) != len(native):
        raise AssertionError("an output is missing rows or has extra ones")
    matched = base.compare(build / "client.jsonl", build / "rust.jsonl")
    rejected = sum(1 for row in native if row["value"].get("error"))
    quads = sum(len(row["value"].get("quads", [])) for row in native)
    base.ensure_sources_unchanged(tested_paths, tested_sources)
    base.ensure_sources_unchanged([inputs], tested_fixtures)
    lab.write_json(build / "report.json", {
        "schema": 1, "minecraft": lock["minecraft"]["version"], "fixtures": len(fixtures), "rejected": rejected,
        "quads": quads, "matched_lines": matched, "client_jar_sha256": lab.digest(jars["client"], "sha256"),
        "sources_sha256": tested_sources, "fixtures_sha256": tested_fixtures,
        "scope": "ResourceMetadata.fromJsonStream then getSection(GuiMetadataSection.TYPE) on each .png.mcmeta text, "
                 "against Sunrise's ui::sprite_metadata_of over a resource source; TiledBlitRenderState.buildVertices with "
                 "UVs from a real TextureAtlasSprite (getU/getV as blitTiledSprite calls them), against ui::tile, "
                 "every quad's positions and UVs as float bits. No GPU, atlas packing or screen layout."})
    print(f"GUI sprite scaling: {matched} lines match ({rejected} rejected metadata, {quads} tiled quads)", flush=True)


if __name__ == "__main__":
    main()
