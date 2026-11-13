#!/usr/bin/env python3
"""Extract world-view values and filtering verbatim; --verify REV checks the donor."""
import argparse
from pathlib import Path
import subprocess

SOURCE = 'crates/horizon-mod-render/src/render.rs'
TARGET = 'crates/horizon-client-render/src/render_views.rs'

def pieces(text):
    start = text.index('/// The most world views')
    end = text.index('/// Asked as each frame', start)
    filtering = text.index('/// The views of those asked')
    filter_end = text.index('pub use horizon_client_render::render_geometry', filtering)
    tests = text.index('#[cfg(test)]', filter_end)
    return text[start:end], text[filtering:filter_end], text[tests:]

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--verify')
parser.add_argument('--output-revision', help='verify the mechanical output before hand corrections')
args = parser.parse_args()
if args.verify:
    donor = subprocess.check_output(['git', 'show', f'{args.verify}:{SOURCE}'], text=True)
    expected = 'use glam::{DVec3, Mat4};\n\n' + '\n'.join(pieces(donor))
    actual = (subprocess.check_output(['git', 'show', f'{args.output_revision}:{TARGET}'], text=True)
              if args.output_revision else Path(TARGET).read_text())
    assert actual.strip() == expected.strip()
else:
    path = Path(SOURCE)
    source = path.read_text()
    moved = pieces(source)
    Path(TARGET).write_text('use glam::{DVec3, Mat4};\n\n' + '\n'.join(moved))
    for piece in moved:
        source = source.replace(piece, '')
    path.write_text(source)
