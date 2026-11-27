//! Structure-template helpers of the structure blocks: the template entry, its size and path checks, the placement settings and the files' encoding.
use std::path::{Path, PathBuf};
use sunrise_core::{
    nbt,
    nbt::{Limits, NamedTag, Tag},
};
use sunrise_world::{
    compression::Compression,
    level::{TemplateSettings, flags},
};

/// A template the manager holds: its file's tree and the author `setAuthor`
/// gave it (the file does not keep one, so a loaded template's is `?`).
#[derive(Clone)]
pub struct TemplateEntry {
    pub tag: Tag,
    pub author: String,
}

/// `StructureTemplate.size` of a template's tree.
pub fn template_size(tag: &Tag) -> [i32; 3] {
    let mut size = [0; 3];
    if let Some(Tag::List(values)) = tag.get("size") {
        for (slot, value) in size.iter_mut().zip(values) {
            if let Tag::Int(v) = value {
                *slot = *v;
            }
        }
    }
    size
}

/// `FileUtil.isPathPortable`-like check of the path under `structures/`: no
/// empty, `.` or `..` segments.
pub fn portable_path(path: &str) -> bool {
    path.split('/')
        .all(|s| !s.is_empty() && s != "." && s != "..")
}

/// `StructureTemplateManager.save`'s file under the world's `generated` folder, or `None` for a path that is not portable.
pub fn template_file(world_root: &Path, name: &str) -> Option<PathBuf> {
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    if !portable_path(path) || !portable_path(ns) {
        return None;
    }
    Some(
        world_root
            .join("generated")
            .join(ns)
            .join("structures")
            .join(format!("{path}.nbt")),
    )
}

/// The gzipped named-tag bytes `StructureTemplateManager.save` writes, or `None` when encoding fails.
pub fn encode_template(tag: &Tag) -> Option<Vec<u8>> {
    let named = NamedTag {
        name: "".into(),
        tag: tag.clone(),
    };
    let raw = nbt::write_named(&named, Limits::templates()).ok()?;
    Compression::Gzip.encode(&raw).ok()
}

/// The `StructurePlaceSettings` that `StructureBlockEntity.placeStructure`
/// and `PlaceCommand.placeTemplate` both build: mirror and rotation, the
/// known shape when strict, the integrity and seed that `place_template`
/// turns into `BlockRotProcessor` and `createRandom(seed)`, and the
/// `setBlock` flags `2 | (strict ? 816 : 0)`.
pub fn load_settings(
    mirror: sunrise_world::worldgen::structure::Mirror,
    rotation: sunrise_world::worldgen::structure::Rotation,
    integrity: f32,
    seed: i64,
    strict: bool,
) -> TemplateSettings {
    TemplateSettings {
        mirror,
        rotation,
        pivot: [0; 3],
        ignore_entities: false,
        known_shape: strict,
        integrity,
        seed: create_seed(seed),
        flags: flags::UPDATE_CLIENTS | if strict { 816 } else { 0 },
    }
}

/// `StructureBlockEntity.createRandom`'s seed: 0 is the clock.
pub fn create_seed(seed: i64) -> i64 {
    if seed == 0 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_millis() as i64)
    } else {
        seed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_must_stay_under_structures() {
        assert!(portable_path("house/small"));
        assert!(!portable_path("../x"));
        assert!(!portable_path("a//b"));
        assert!(!portable_path("a/./b"));
        assert!(!portable_path(""));
    }

    #[test]
    fn template_size_reads_the_size_list() {
        let tag = Tag::compound([(
            "size",
            Tag::List(vec![Tag::Int(3), Tag::Int(4), Tag::Int(5)]),
        )]);
        assert_eq!(template_size(&tag), [3, 4, 5]);
        assert_eq!(
            template_size(&Tag::compound(Vec::<(&str, Tag)>::new())),
            [0; 3]
        );
    }

    #[test]
    fn a_zero_seed_is_the_clock() {
        assert_eq!(create_seed(42), 42);
        assert_ne!(create_seed(0), 0);
    }

    #[test]
    fn template_settings_are_flags_2_and_unknown_shape_unless_strict() {
        use sunrise_world::worldgen::structure::{Mirror, Rotation};
        let loose = load_settings(Mirror::LeftRight, Rotation::Clockwise90, 0.5, 7, false);
        assert_eq!(loose.flags, 2);
        assert!(!loose.known_shape);
        assert!(!loose.ignore_entities);
        assert_eq!((loose.integrity, loose.seed), (0.5, 7));
        assert_eq!(
            (loose.mirror, loose.rotation),
            (Mirror::LeftRight, Rotation::Clockwise90)
        );
        let strict = load_settings(Mirror::None, Rotation::None, 1.0, 7, true);
        assert_eq!(strict.flags, 818);
        assert!(strict.known_shape);
    }
}
