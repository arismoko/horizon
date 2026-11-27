//! Client screen adapter for Create World's Data Packs screen; all portable data-pack
//! discovery, parsing, and loading lives in `sunrise_level::datapack`.
use std::path::Path;
pub use sunrise_level::datapack::*;

// ---- Create World ------------------------------------------------------------

/// Create World's Data Packs screen (`PackSelectionScreen` on vanilla's
/// `mcworld-` folder): the folder's packs (`PackSource.WORLD`), the
/// built-in one (`resources` holds its `pack.png`), and the feature packs
/// of `resources` (`PackSource.FEATURE`), available at the top and off
/// until picked — the Experiments.
pub fn screen_packs(dir: &Path, resources: &Path) -> Vec<crate::pack_screen::PackInfo> {
    use crate::{
        pack_screen::{Compat, PackInfo, Source, load_icon},
        packs::Position,
        text::{Style, Text},
    };
    let format = sunrise_core::version::DATA_PACK_FORMAT;
    let mut infos: Vec<PackInfo> = crate::packs::folder(dir, format)
        .into_iter()
        .map(|p| PackInfo {
            icon: p.icon_png().as_deref().and_then(load_icon),
            id: p.id,
            title: p.title,
            description: p.raw_description,
            description_style: p.description_style,
            source: Source::World,
            compat: p.compatibility,
            required: false,
            position: Position::Top,
            fixed: false,
            hidden: false,
        })
        .collect();
    infos.push(PackInfo {
        id: "vanilla".into(),
        title: Text::translate("dataPack.vanilla.name", &[]),
        description: Text::translate("dataPack.vanilla.description", &[]),
        description_style: Style::default(),
        source: Source::BuiltIn,
        compat: Compat::Compatible,
        icon: std::fs::read(resources.join("pack.png"))
            .ok()
            .as_deref()
            .and_then(load_icon),
        required: false,
        position: Position::Bottom,
        fixed: false,
        hidden: false,
    });
    for p in feature_packs(resources) {
        infos.extend(p.selection);
    }
    infos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_world_installs_its_packs() {
        let root = std::env::temp_dir().join(format!("hz-datapack-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, world) = (root.join("mcworld"), root.join("world"));
        let write = |pack: &str, p: &str, t: &str| {
            let path = dir.join(pack).join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "fns",
            "pack.mcmeta",
            r#"{"pack":{"description":"fixture","min_format":121,"max_format":121}}"#,
        );
        write("fns", "data/x/function/f.mcfunction", "say hi");
        write(
            "off",
            "pack.mcmeta",
            r#"{"pack":{"description":"fixture","min_format":121,"max_format":121}}"#,
        );
        std::fs::create_dir_all(&dir).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(dir.join("old.zip")).unwrap());
        zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, br#"{"pack":{"pack_format":48}}"#).unwrap();
        zip.finish().unwrap();
        let infos = screen_packs(&dir, &root);
        let ids: Vec<&str> = infos.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["file/fns", "file/off", "file/old.zip", "vanilla"]);
        let compat: Vec<_> = infos.iter().map(|p| p.compat).collect();
        use crate::pack_screen::Compat;
        assert_eq!(
            compat,
            [
                Compat::Compatible,
                Compat::Compatible,
                Compat::TooOld,
                Compat::Compatible
            ]
        );
        let selected = ["vanilla", "file/old.zip", "file/fns"].map(String::from);
        install(&dir, &world, &selected).unwrap();
        let packs = discover(&world);
        let ids: Vec<&str> = packs.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["file/fns", "file/off", "file/old.zip"]);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(world.join("datapacks.json")).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({"enabled": selected, "disabled": ["file/off"]})
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
