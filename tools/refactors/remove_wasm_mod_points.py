"""Remove wasm mod-to-mod point bridges; retain engine point forwarding."""
from pathlib import Path
root = Path(__file__).resolve().parents[2]
p = root / 'crates/horizon-mod-wasm/src/runtime.rs'
s = p.read_text(); a = s.index('        // Register through the library'); b = s.index('        let lending =', a); p.write_text(s[:a] + s[b:])
p = root / 'crates/horizon-mod-wasm/src/runtime_tests.rs'
s = p.read_text(); s = s[:s.index('#[test]\nfn registration_refusal_names_the_indexed_mod_injection')]; p.write_text(s)
p = root / 'crates/horizon-mod-wasm/src/points.rs'
s = p.read_text().replace('        .filter(|inject| manifest.mod_point_dependency(&inject.at).is_none())\n', ''); p.write_text(s)
p = root / 'crates/horizon-mod-wasm/src/lib.rs'
s = p.read_text().replace('                    && package.manifest.mod_point_dependency(&inject.at).is_none()\n', ''); p.write_text(s)
p = root / 'crates/horizon-mod-wasm/src/link.rs'
s = p.read_text().replace('                            | "horizon_mod_points_register"\n', '').replace('                            | "horizon_mod_points_seal"\n', ''); p.write_text(s)
p = root / 'crates/horizon-mod-points/src/guest.rs'
s = p.read_text(); a = s.index('    if !std::ptr::eq'); b = s.index('    if let Some', a); s = s[:a] + s[b:]; s = s.replace("/// table: the host registers the subscribed places in its ordinary ordered listener list. A\n/// listener at a mod's own point registers in that mod's table, in the group's own memory.", '/// table: the host registers the subscribed places in its ordinary ordered listener list.'); p.write_text(s)
(root / 'crates/horizon-mod-wasm/tests/fixture/mod-points.json').unlink(missing_ok=True)
