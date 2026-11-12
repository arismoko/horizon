#!/usr/bin/env python3
"""Remove automatic mod-to-mod point instrumentation; engine API points remain."""
from pathlib import Path
root = Path('launcher/src')
p = root / 'native_mods.rs'
p.write_text(p.read_text().replace('mod mod_points;\n', ''))
(root / 'native_mods/mod_points.rs').unlink()
for name in ['api.rs', 'prebuilt.rs', 'registry_compile.rs', 'planning.rs']:
    p = root / 'native_mods' / name
    s = p.read_text()
    s = s.replace('        .chain(super::mod_points::emitted(catalog, mods))\n', '')
    s = s.replace('        super::mod_points::stage(&dest, &catalog.wire)?;\n', '')
    s = s.replace('        super::mod_points::stage(&staged, &catalog.wire)?;\n', '')
    s = s.replace('?.with_mods(projects.iter().map(|project| project.bundle))?', '?')
    s = s.replace('?.with_mods(projects.iter().map(|p| p.bundle))?', '?')
    s = s.replace('                let catalog = catalog.with_mods(projects.iter().map(|p| p.bundle))?;\n', '')
    p.write_text(s)
p = root / 'native_mods/internals.rs'
s = p.read_text().replace('    pub(super) mods: Vec<super::mod_points::ModLibrary>,\n', '').replace('            mods: Vec::new(),\n', '').replace('        super::mod_points::routes(self, manifest).map_err(|e| Error::Inject(e.to_string()))?;\n', '')
p.write_text(s)
p = root / 'native_mods/slots.rs'
s = p.read_text().replace('        if catalog.mod_owner(&entry.at).is_some() {\n            continue;\n        }\n', '')
p.write_text(s)
