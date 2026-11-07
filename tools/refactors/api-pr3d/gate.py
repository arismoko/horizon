#!/usr/bin/env python3
"""Compare generated raw event records, WIT and host manifests for PR 3d."""
import argparse
from pathlib import Path
import re

EVENTS = ("HandDrawnEvent", "RenderNeedsEvent", "WorldDrawnEvent")


def records(source):
    if re.search(r"#(?:\s|/\*[\s\S]*?\*/|//[^\n]*(?:\n|$))*!", source):
        raise ValueError("inner attributes are unsupported in raw record source")
    # This byte gate supports only the generator's contiguous, single-line attributes.
    # Refuse other Rust layouts rather than silently dropping an attribute from the proof.
    for start in re.finditer(r"#\s*\[", source):
        attribute = re.match(r"#\[[^\n]*\][ \t]*\n", source[start.start():])
        if attribute is None:
            if re.match(r"#\[doc\(hidden\)\] (?:pub )?fn\b", source[start.start():]):
                continue  # The old generator also annotates dispatch functions inline.
            raise ValueError("unsupported attribute layout in raw record source")
        following = source[start.start() + attribute.end():].lstrip()
        if following.startswith(("//", "/*")):
            raise ValueError("comment-separated attributes are unsupported")
    found = {}
    for match in re.finditer(r"(?:#\[[^\n]*\][ \t]*\n[ \t\n]*)*pub struct (\w+Event)([^{}]*)\{([^{}]*)\}", source):
        name = match.group(1)
        if name in EVENTS:
            if name in found:
                raise ValueError(f"duplicate raw record: {name}")
            found[name] = match.group(0)
    if set(found) != set(EVENTS):
        raise ValueError(f"missing raw records: {set(EVENTS) - set(found)}")
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("render", "wit", "host"):
        parser.add_argument(f"--before-{name}", type=Path, required=True)
        parser.add_argument(f"--after-{name}", type=Path, required=True)
    args = parser.parse_args()
    if records(args.before_render.read_text()) != records(args.after_render.read_text()):
        raise ValueError("raw record bytes changed")
    for name in ("wit", "host"):
        before = getattr(args, f"before_{name}").read_bytes()
        after = getattr(args, f"after_{name}").read_bytes()
        if before != after:
            raise ValueError(f"{name} bytes changed: report the exact delta before proceeding")
    print("PASS: raw record definitions, WIT and host manifest byte-identical")


if __name__ == "__main__":
    main()
