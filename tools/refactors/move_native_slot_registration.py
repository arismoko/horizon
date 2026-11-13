#!/usr/bin/env python3
"""Move API listener registration verbatim from engine entry to host loading.

Run once from the repository root. Seal socket integration is a separate hand edit.
"""
from pathlib import Path
entry = Path('crates/horizon-client/src/entry.rs')
host = Path('crates/horizon-mod-host/src/loading.rs')
e = entry.read_text()
h = host.read_text()
start = e.index('    for id in ids {')
end = e.index('    crash::install', start)
registration = e[start:end]
e = e[:start] + e[end:]
e = e.replace('/// A native function slot, checked against its place\'s exact type before erasure.\npub use mod_points::__private::SlotListener as NativeSlot;\n\n', '')
e = e.replace('/// The places listeners register at, named by the generated `main`.\n#[doc(hidden)]\npub use mod_points::PointId;\n\n', '')
e = e.replace('    slots: &[NativeSlot],\n', '')
e = e.replace('let (ids, _loaded_code)', 'let (_ids, _loaded_code)')
h = h.replace('horizon_client::entry::NativeSlot', 'crate::SlotListener')
h = h.replace('            (ids, wasm)', registration.replace('    ', '            ', 1) + '            (ids, wasm)')
h = h.replace('        slots,\n', '')
entry.write_text(e)
host.write_text(h)
p = Path('crates/horizon-client/src/native_main.rs.template')
p.write_text(p.read_text().replace('horizon_client::entry::NativeSlot', 'horizon_mod_host::SlotListener'))
