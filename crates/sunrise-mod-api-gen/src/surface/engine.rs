//! Project SDK staging's source catalogs into the surface without rewriting the tree.
use crate::SourceLayout;
use mod_surface::{EngineContract, EngineItem};
use std::path::Path;

/// Reads the published loops and, for a full audit, the advisory internals of a source tree.
pub fn collect(root: &Path, with_internals: bool) -> Result<Vec<EngineItem>, String> {
    collect_with_layout(root, SourceLayout::read(root)?, with_internals)
}

pub fn collect_with_layout(
    root: &Path,
    layout: SourceLayout,
    with_internals: bool,
) -> Result<Vec<EngineItem>, String> {
    let mut items = Vec::new();
    for published in crate::loops::collect_with_layout(root, layout)? {
        items.push(EngineItem::new(
            format!("loop:{}", published.name),
            EngineContract::Loop {
                entry: published.entry,
                generic: published.generic,
                slot: published.slot,
                side: published.side,
            },
        ));
    }
    if !with_internals {
        return Ok(items);
    }
    for internal in crate::internals::rule_catalog_with_layout(root, layout)? {
        let mut id = format!("internal:{}:{}", internal.kind, internal.path);
        if !internal.cfg.is_empty() {
            id.push_str(&format!(" [{}]", internal.cfg));
        }
        items.push(EngineItem::new(
            id,
            EngineContract::Internal {
                signature: internal.signature,
                signature_hash: internal.hash,
                shape_only: internal.shape_only,
                opaque_reason: internal.opaque_reason.map(str::to_owned),
            },
        ));
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn exports_loops_from_source_with_opt_in_internals() {
        let tree = tempfile::tempdir().unwrap();
        let root = tree.path();
        let write = |file: &str, text: &str| {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let game = "crates/sunrise-client/src/game.rs";
        let source = r#"
fn tick() {}
#[cfg(unix)] fn helper() {}
struct Game;
impl Game {
    #[sunrise_mod_api_macros::sunrise_loop(name = "things.tick", side = "server")]
    fn tick_things(&mut self) { self.tick_things_with(&mut ()) }
    fn tick_things_with<S: ThingSlot>(&mut self, _slot: &mut S) {}
}
"#;
        write(game, source);
        let default = collect(root, false).unwrap();
        assert_eq!(
            default
                .iter()
                .find(|item| item.id == "loop:things.tick")
                .map(|item| &item.contract),
            Some(&EngineContract::Loop {
                entry: "sunrise_client::game::Game::tick_things".into(),
                generic: "tick_things_with".into(),
                slot: "ThingSlot".into(),
                side: "server".into(),
            })
        );

        write("crates/sunrise-client/Cargo.toml", "[dependencies]\n");
        write("crates/sunrise-client/src/entry.rs", "");
        write("crates/sunrise-mod-api-gen/internals.txt", "");
        write(crate::crossing::CROSSING_FOREIGN_FILE, "[]");
        let audit = collect(root, true).unwrap();
        assert_eq!(&audit[..default.len()], default.as_slice());
        assert!(
            audit
                .iter()
                .any(|item| item.id == "internal:fn:sunrise_client::game::helper [unix]")
        );

        write(game, &source.replace("things.tick", "things.step"));
        let surface = |items| {
            mod_surface::Surface::new(
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .with_engine_items(items)
        };
        let renamed = surface(collect(root, false).unwrap());
        let diff = mod_surface::diff(&surface(default), &renamed);
        assert_eq!(
            diff.undeclared()
                .map(|change| change.id.as_str())
                .collect::<Vec<_>>(),
            ["loop:things.tick"]
        );
    }
}
