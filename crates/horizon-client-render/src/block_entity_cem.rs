//! OptiFine custom entity models over the block entities' models (`block_entity_render`'s
//! `MODELS`): `chest.jem`, `shulker_box.jem`, `banner.jem`, `bell.jem`, `head_zombie.jem`... (the
//! "Block entity name" table of OptiFine's `cem_model.txt`, and Entity Model Features' one-file
//! `banner`), found and read as the mobs' are ([`horizon_optifine::cem::files`]). Each entry
//! replaces or joins the boxes of the vanilla part it names and takes its `scale`; animations
//! don't run yet. A `.jem`'s texture paints its model instead of vanilla's when the atlas has it.
//!
//! Trapped and ender chests take their own file, else the chest's geometry (not its texture);
//! copper chests take the chest's geometry. Lectern and enchanting-table books take theirs, else
//! `book`. Signs, hanging signs and beds are block models here, so their files are not read;
//! copper golem statues have no CEM name.

use crate::block_entity_render::ModelPart;
use horizon_optifine::cem::{
    files::{atlas_name, jem_files, load_jem},
    model::{Bone, EntityModel},
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

/// One model the renderer draws, the CEM names that reshape it, best first, and OptiFine's
/// names for its vanilla parts.
struct Target {
    key: &'static str,
    /// The vanilla model it starts from: `key` itself, or the one a block the renderer tells
    /// apart shares (a trapped chest's is the chest's).
    base: &'static str,
    names: &'static [&'static str],
    /// A model whose geometry it takes last, without its texture.
    borrow: Option<&'static str>,
    parts: &'static [(&'static str, &'static str)],
}

const CHEST: &[(&str, &str)] = &[
    ("root", "root"),
    ("base", "bottom"),
    ("lid", "lid"),
    ("knob", "lock"),
];
const SHULKER: &[(&str, &str)] = &[("root", "root"), ("base", "base"), ("lid", "lid")];
const BANNER: &[(&str, &str)] = &[("stand", "pole"), ("top", "bar")];
const FLAG: &[(&str, &str)] = &[("slate", "flag")];
const HEAD: &[(&str, &str)] = &[("root", "root"), ("head", "head")];
const DRAGON: &[(&str, &str)] = &[("root", "root"), ("head", "head"), ("jaw", "jaw")];
const PIGLIN: &[(&str, &str)] = &[
    ("root", "root"),
    ("head", "head"),
    ("left_ear", "left_ear"),
    ("right_ear", "right_ear"),
];
const BELL: &[(&str, &str)] = &[
    ("root", "root"),
    ("body", "bell_body"),
    ("base", "bell_base"),
];
const BOOK: &[(&str, &str)] = &[
    ("root", "root"),
    ("cover_right", "right_lid"),
    ("cover_left", "left_lid"),
    ("pages_right", "right_pages"),
    ("pages_left", "left_pages"),
    ("flipping_page_right", "flip_page1"),
    ("flipping_page_left", "flip_page2"),
    ("book_spine", "seam"),
];
const POT_BASE: &[(&str, &str)] = &[("neck", "neck"), ("top", "top"), ("bottom", "bottom")];
const POT_SIDES: &[(&str, &str)] = &[
    ("front", "front"),
    ("back", "back"),
    ("left", "left"),
    ("right", "right"),
];

const fn model(
    key: &'static str,
    names: &'static [&'static str],
    parts: &'static [(&'static str, &'static str)],
) -> Target {
    Target {
        key,
        base: key,
        names,
        borrow: None,
        parts,
    }
}
const fn copy(
    key: &'static str,
    base: &'static str,
    names: &'static [&'static str],
    borrow: Option<&'static str>,
    parts: &'static [(&'static str, &'static str)],
) -> Target {
    Target {
        key,
        base,
        names,
        borrow,
        parts,
    }
}

/// In order: a copy comes after its base.
const TARGETS: &[Target] = &[
    model("chest", &["chest"], CHEST),
    model("double_chest_left", &["chest_left"], CHEST),
    model("double_chest_right", &["chest_right"], CHEST),
    copy(
        "trapped_chest",
        "chest",
        &["trapped_chest"],
        Some("chest"),
        CHEST,
    ),
    copy(
        "trapped_chest_left",
        "double_chest_left",
        &["trapped_chest_left"],
        Some("chest_left"),
        CHEST,
    ),
    copy(
        "trapped_chest_right",
        "double_chest_right",
        &["trapped_chest_right"],
        Some("chest_right"),
        CHEST,
    ),
    copy(
        "ender_chest",
        "chest",
        &["ender_chest"],
        Some("chest"),
        CHEST,
    ),
    model("shulker_box", &["shulker_box"], SHULKER),
    model("standing_banner", &["banner_base", "banner"], BANNER),
    model("wall_banner", &["banner_base", "banner"], BANNER),
    model("standing_banner_flag", &["banner_flag", "banner"], FLAG),
    model("wall_banner_flag", &["banner_flag", "banner"], FLAG),
    model("skeleton_skull", &["head_skeleton"], HEAD),
    model("wither_skeleton_skull", &["head_wither_skeleton"], HEAD),
    model("zombie_head", &["head_zombie"], HEAD),
    model("creeper_head", &["head_creeper"], HEAD),
    model("dragon_skull", &["head_dragon"], DRAGON),
    model("piglin_head", &["head_piglin"], PIGLIN),
    model("player_head", &["head_player"], HEAD),
    model("bell", &["bell"], BELL),
    model("book", &["enchanting_book", "book"], BOOK),
    copy(
        "lectern_book",
        "book",
        &["lectern_book", "book"],
        None,
        BOOK,
    ),
    model("conduit_shell", &["conduit"], &[("base", "shell")]),
    model("conduit_cage", &["conduit"], &[("cage", "shell")]),
    model("conduit_wind", &["conduit"], &[("wind", "wind")]),
    model("conduit_eye", &["conduit"], &[("eye", "eye")]),
    model("decorated_pot_base", &["decorated_pot"], POT_BASE),
    model("decorated_pot_sides", &["decorated_pot"], POT_SIDES),
];

/// Every model, reshaped or vanilla's, and the atlas names of the `.jem` textures by model.
pub type Reshaped = (
    HashMap<&'static str, ModelPart>,
    HashMap<&'static str, String>,
);

/// The models the packs under `root` reshape, over `vanilla`'s; None when no block entity has
/// a `.jem` there.
pub fn reshape(root: &Path, vanilla: &HashMap<&'static str, ModelPart>) -> Option<Reshaped> {
    let files = jem_files(root);
    let named = |t: &Target| {
        t.names
            .iter()
            .chain(&t.borrow)
            .any(|n| files.contains_key(*n))
    };
    if !TARGETS.iter().any(named) {
        return None;
    }
    let mut models = vanilla.clone();
    let mut textures = HashMap::new();
    let mut reshaped = HashSet::new();
    // Each file parsed once; which of its entries found a part in some model.
    let mut parsed: HashMap<&str, Option<EntityModel>> = HashMap::new();
    let mut used: HashMap<&str, Vec<bool>> = HashMap::new();
    for t in TARGETS {
        let Some(model) = vanilla.get(t.base) else {
            continue;
        };
        let names = t.names.iter().map(|n| (*n, true));
        let found = names
            .chain(t.borrow.map(|n| (n, false)))
            .flat_map(|(name, own)| {
                files
                    .get(name)
                    .into_iter()
                    .flatten()
                    .map(move |path| (own, path.as_str()))
            })
            .find(|&(_, path)| {
                parsed
                    .entry(path)
                    .or_insert_with(|| load_jem(root, path))
                    .is_some()
            });
        let Some((own, path)) = found else {
            // A copy of a reshaped base that has no file of its own stays vanilla.
            if t.key != t.base && reshaped.contains(t.base) {
                models.insert(t.key, model.clone());
            }
            continue;
        };
        let Some(Some(jem)) = parsed.get(path) else {
            continue;
        };
        let mut out = model.clone();
        let hits = apply(&mut out, jem, t.parts);
        let marks = used
            .entry(path)
            .or_insert_with(|| vec![false; jem.parts.len()]);
        for (mark, hit) in marks.iter_mut().zip(hits) {
            *mark |= hit;
        }
        models.insert(t.key, out);
        reshaped.insert(t.key);
        if own && let Some(texture) = jem.texture.as_deref().and_then(atlas_name) {
            textures.insert(t.key, texture);
        }
    }
    for (path, marks) in &used {
        let Some(Some(jem)) = parsed.get(path) else {
            continue;
        };
        for (p, _) in jem.parts.iter().zip(marks).filter(|(_, used)| !**used) {
            log::warn!("Horizon: custom entity model {path}: no part {}", p.part);
        }
    }
    Some((models, textures))
}

/// `jem`'s entries on `model`, OptiFine's part names read through `parts`: a replacing entry
/// clears its part's boxes, and each joins the part as a child `cem#<n>`. Which entries found
/// their part.
fn apply(model: &mut ModelPart, jem: &EntityModel, parts: &[(&str, &str)]) -> Vec<bool> {
    let mut count = 0;
    let mut hits = Vec::with_capacity(jem.parts.len());
    for p in &jem.parts {
        let Some(&(_, vanilla)) = parts.iter().find(|(of, _)| *of == p.part) else {
            hits.push(false);
            continue;
        };
        let Some(node) = find_mut(model, vanilla) else {
            hits.push(false);
            continue;
        };
        if !p.attach {
            node.polygons.clear();
        }
        let (name, mut bone) = part(&p.bone, &mut count);
        bone.pose[6..9].fill(p.scale);
        node.children.push((name, bone));
        hits.push(true);
    }
    hits
}

/// The model itself for `root`, else its first part named `name`, depth first.
fn find_mut<'a>(model: &'a mut ModelPart, name: &str) -> Option<&'a mut ModelPart> {
    fn child<'a>(part: &'a mut ModelPart, name: &str) -> Option<&'a mut ModelPart> {
        part.children.iter_mut().find_map(|(n, c)| {
            if n.as_str() == name {
                Some(c)
            } else {
                child(c, name)
            }
        })
    }
    if name == "root" {
        return Some(model);
    }
    child(model, name)
}

/// A baked bone as a model part, named `cem#<n>`: unique in the model, so no vanilla pose
/// change by name reaches it.
fn part(bone: &Bone, count: &mut usize) -> (String, ModelPart) {
    let name = format!("cem#{count}");
    *count += 1;
    let [x, y, z] = bone.pivot;
    let [rx, ry, rz] = bone.rotation;
    let out = ModelPart {
        pose: [x, y, z, rx, ry, rz, 1.0, 1.0, 1.0],
        polygons: bone.quads.iter().map(|q| q.vertices).collect(),
        children: bone.children.iter().map(|c| part(c, count)).collect(),
    };
    (name, out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    /// A part with one face and `children`.
    pub(crate) fn plane(children: Vec<(&str, ModelPart)>) -> ModelPart {
        ModelPart {
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![[[0.0; 5]; 4]],
            children: children
                .into_iter()
                .map(|(n, c)| (n.to_owned(), c))
                .collect(),
        }
    }
    pub(crate) fn chest() -> ModelPart {
        let mut root = plane(vec![
            ("bottom", plane(vec![])),
            ("lid", plane(vec![])),
            ("lock", plane(vec![])),
        ]);
        root.polygons.clear();
        root
    }
    pub(crate) const BOX: &str =
        r#""boxes": [{"coordinates": [0, 0, 0, 1, 1, 1], "textureOffset": [0, 0]}]"#;
    /// A fresh pack folder named for `test`, holding `files` under `assets/minecraft/`.
    pub(crate) fn pack(test: &str, files: &[(&str, String)]) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-be-cem-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join("assets/minecraft").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        root
    }
    /// A `.jem` of one entry on `part`.
    fn jem(part: &str) -> String {
        format!(r#"{{"models": [{{"part": "{part}", {BOX}}}]}}"#)
    }

    #[test]
    fn a_chest_jem_replaces_and_joins_parts_and_trapped_chests_borrow_it() {
        let root = pack(
            "chest",
            &[(
                "optifine/cem/chest.jem",
                format!(
                    r#"{{"texture": "textures/entity/chest/fancy.png", "models": [
                        {{"part": "lid", {BOX}}},
                        {{"part": "knob", "attach": true, "scale": 2, {BOX}}},
                        {{"part": "tail", {BOX}}}]}}"#
                ),
            )],
        );
        let vanilla: HashMap<_, _> = [("chest", chest()), ("double_chest_left", chest())].into();
        let (models, textures) = reshape(&root, &vanilla).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let chest = &models["chest"];
        // Replaced: the lid's own face goes and the custom box (6 faces) joins it.
        let lid = chest.child("lid").unwrap();
        assert_eq!(lid.polygons.len(), 0);
        assert_eq!(lid.child("cem#0").unwrap().polygons.len(), 6);
        // Attached: the lock keeps its face beside the scaled box.
        let lock = chest.child("lock").unwrap();
        assert_eq!(lock.polygons.len(), 1);
        assert_eq!(lock.child("cem#1").unwrap().pose[6..], [2.0, 2.0, 2.0]);
        // `tail` names no chest part: nothing else changed.
        let bottom = chest.child("bottom").unwrap();
        assert_eq!((bottom.polygons.len(), bottom.children.len()), (1, 0));
        // The double chest has no file; trapped and ender chests take the geometry only.
        assert_eq!(
            models["double_chest_left"]
                .child("lid")
                .unwrap()
                .polygons
                .len(),
            1
        );
        for key in ["trapped_chest", "ender_chest"] {
            assert_eq!(models[key].child("lid").unwrap().polygons.len(), 0, "{key}");
        }
        assert_eq!(
            textures,
            HashMap::from([("chest", "entity/chest/fancy".to_owned())])
        );
    }

    #[test]
    fn banners_split_one_file_and_books_fall_back() {
        let root = pack(
            "banner",
            &[
                (
                    "optifine/cem/banner.jem",
                    format!(
                        r#"{{"models": [{{"part": "stand", {BOX}}}, {{"part": "top", {BOX}}},
                            {{"part": "slate", {BOX}}}]}}"#
                    ),
                ),
                ("emf/cem/enchanting_book.jem", jem("cover_left")),
            ],
        );
        let banner = plane(vec![("pole", plane(vec![])), ("bar", plane(vec![]))]);
        let wall = plane(vec![("bar", plane(vec![]))]);
        let flag = plane(vec![("flag", plane(vec![]))]);
        let book = plane(vec![("left_lid", plane(vec![]))]);
        let vanilla: HashMap<_, _> = [
            ("standing_banner", banner),
            ("wall_banner", wall),
            ("wall_banner_flag", flag),
            ("book", book),
        ]
        .into();
        let (models, textures) = reshape(&root, &vanilla).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let replaced = |key: &str, part: &str| {
            let p = models[key].child(part).unwrap();
            (p.polygons.len(), p.children.len())
        };
        assert_eq!(replaced("standing_banner", "pole"), (0, 1));
        assert_eq!(replaced("standing_banner", "bar"), (0, 1));
        assert_eq!(replaced("wall_banner", "bar"), (0, 1));
        assert_eq!(replaced("wall_banner_flag", "flag"), (0, 1));
        assert_eq!(replaced("book", "left_lid"), (0, 1));
        // The lectern's book has no file of its own nor `book.jem`: vanilla's.
        assert_eq!(
            models["lectern_book"]
                .child("left_lid")
                .unwrap()
                .polygons
                .len(),
            1
        );
        assert!(textures.is_empty());
    }
}
