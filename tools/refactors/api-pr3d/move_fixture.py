#!/usr/bin/env python3
"""Move the GPU fixture out of Cargo's unstaged examples; check its exact preservation."""
import argparse
from pathlib import Path
import subprocess

OLD = "crates/horizon-client-render/examples/standard_pack_gpu_entities.rs"
NEW = "crates/horizon-client-render/fixtures/standard_pack_gpu_entities.rs"
BEFORE = "//! `cargo run -p horizon-client-render --example standard_pack_gpu_entities -- <output-dir>`"
AFTER = "//! `python3 tools/run_standard_pack_gpu_entities.py <output-dir>`"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", metavar="BASE")
    args = parser.parse_args()
    if args.check:
        before = subprocess.check_output(["git", "show", f"{args.check}:{OLD}"]).decode()
        if Path(OLD).exists() or before.replace(BEFORE, AFTER) != Path(NEW).read_text():
            raise ValueError("fixture changed beyond its declared run-command edit")
        print("PASS: fixture preserved exactly apart from its declared run command")
    else:
        text = Path(OLD).read_text()
        if Path(NEW).exists() or text.count(BEFORE) != 1:
            raise ValueError("unexpected fixture source or destination")
        Path(NEW).parent.mkdir(parents=True, exist_ok=True)
        Path(NEW).write_text(text.replace(BEFORE, AFTER))
        Path(OLD).unlink()


if __name__ == "__main__":
    main()
