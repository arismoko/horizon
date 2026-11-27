//! The game's generated host forwarders, written into a source copy for tools that can't read OUT_DIR.
use std::{fs, path::Path};

const HOST: &str = "include!(concat!(env!(\"OUT_DIR\"), \"/game_host_generated.rs\"));";

/// `source` with the host forwarders' include replaced by the generated items, without needing OUT_DIR.
/// Other includes are left alone: source tools must not evaluate arbitrary build-time code.
pub(crate) fn expand(root: &Path, path: &Path, source: &str) -> Result<String, String> {
    if !source.contains(HOST) {
        return Ok(source.to_owned());
    }
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut lines: Vec<_> = source.split_inclusive('\n').map(str::to_owned).collect();
    for item in file.items {
        if let syn::Item::Macro(item) = item
            && item.mac.path.is_ident("include")
        {
            let line = item.mac.path.segments[0].ident.span().start().line - 1;
            if lines[line].trim_end_matches(['\r', '\n']) != HOST {
                continue;
            }
            let physical_root = root.canonicalize().ok();
            let package = crate::GameplayPackage::ALL.into_iter().find(|package| {
                let domain = package.domain();
                path.starts_with(domain.package_path(root))
                    || physical_root
                        .as_ref()
                        .is_some_and(|root| path.starts_with(domain.package_path(root)))
            });
            let generated = match package {
                Some(package) => crate::gameplay::host_source(root, package)?,
                None => crate::build::game_host_generated(root)?,
            };
            let generated = syn::parse_file(&generated).map_err(|e| e.to_string())?;
            lines[line] = prettyplease::unparse(&generated);
        }
    }
    Ok(lines.concat())
}

/// Write the generated host forwarders into a disposable game source copy before external tools
/// traverse it, without widening visibility or requiring an OUT_DIR. Never pass a downloaded cache.
pub fn materialize_generated_host(root: &Path) -> Result<(), String> {
    materialize_with_layout(root, crate::SourceLayout::read(root)?)
}

fn materialize_with_layout(root: &Path, layout: crate::source::SourceLayout) -> Result<(), String> {
    for file in layout.files(root, false)? {
        let source =
            fs::read_to_string(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
        let expanded = expand(root, &file.path, &source)?;
        if expanded != source {
            fs::write(&file.path, expanded).map_err(|e| format!("{}: {e}", file.path.display()))?;
        }
    }
    Ok(())
}

pub(crate) fn parse(root: &Path, path: &Path, source: &str) -> Result<syn::File, String> {
    syn::parse_file(&expand(root, path, source)?).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Git for Windows checks sources out with CRLF by default.
    #[test]
    fn the_host_include_becomes_its_generated_items_whatever_ends_its_line() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for end in ["\n", "\r\n"] {
            let source = format!("pub(crate) mod draw;{end}{HOST}{end}");
            let file =
                syn::parse_file(&expand(&root, Path::new("game/host/mod.rs"), &source).unwrap())
                    .unwrap();
            let names: Vec<_> = file
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Mod(module) => Some(module.ident.to_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(names, ["draw", "generated"], "{end:?}");
        }
        let other = "include!(concat!(env!(\"OUT_DIR\"), \"/mod_api_fire_generated.rs\"));\n";
        assert_eq!(
            expand(&root, Path::new("game/native_hooks/fire.rs"), other).unwrap(),
            other
        );
    }
}
