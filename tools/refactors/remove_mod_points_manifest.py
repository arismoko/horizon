from pathlib import Path
import subprocess
for file in ['manifest.rs','manifest/wasm.rs','manifest/mixin.rs','layouts.rs']:
 p=Path('crates/horizon-mod-package/src')/file
 p.write_bytes(subprocess.check_output(['git','show','origin/no-points:'+str(p)]))
p=Path('crates/horizon-mod-package/src/manifest.rs');s=p.read_text().replace('mod jar;','pub mod inject;\nmod jar;');s=s.replace('    pub internals: Vec<Internal>,','    pub internals: Vec<Internal>,\n    #[serde(default, deserialize_with = "inject::rows")]\n    pub inject: Vec<inject::Inject>,');a=s.index('        Internal::check_all');s=s[:a]+'''        inject::Inject::check_all(&m.inject, &m.id, m.native.is_some() || m.wasm.is_some()).map_err(|e| e.to_string())?;
        for entry in &m.inject {
            if m.dependencies.keys().any(|id| entry.at.split("::").next() == Some(id.replace('-', "_").as_str())) {
                return Err(format!("{}: target {} is another mod's function; use the library mod's own hooks and functions", entry.id, entry.at));
            }
        }
'''+s[a:];p.write_text(s)
p=Path('crates/horizon-mod-package/src/manifest/inject.rs');s=p.read_text().replace('pub mod mod_points;','');p.write_text(s)
p=Path('crates/cargo-horizon-mod/src/inject.rs');s=p.read_text();a=s.index('    if let Some(library)');b=s.index('    let candidate = append(',a);s=s[:a]+s[b:];a=s.index('/// The points of the dependency');b=s.index('fn append(\n',a);s=s[:a]+s[b:];a=s.index('    /// The `mod_api`');b=s.index('\n}',a);s=s[:a]+s[b:];p.write_text(s)
p=Path('crates/cargo-horizon-mod/src/inject_update.rs');s=p.read_text().replace('let mut internals =','let internals =');a=s.index('    // A declaration at');b=s.index('    // Older cached',a);s=s[:a]+s[b:];a=s.index('        if let Some(catalog) = super::library');b=s.index('            let item = target',a);s=s[:a]+s[b:];a=s.index('\n        }\n    }\n    Ok(())',s.index('pub(crate) fn sync'));s=s[:a]+s[a:].replace('\n        }','',1);p.write_text(s)
