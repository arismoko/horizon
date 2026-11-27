use super::{Content, ContentMod, J, Result};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn prepared() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") {
        let root = PathBuf::from(root);
        assert!(
            root.join("gameplay-facts.json").is_file(),
            "SUNRISE_TEST_RESOURCES is not prepared"
        );
        return Some(root);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("../../.local/sunrise-client/26.3"),
        manifest.join("../../../../.local/sunrise-client/26.3"),
    ]
    .into_iter()
    .find(|root| root.join("gameplay-facts.json").exists())
}

/// Exercise relative prepared roots even when the data lives outside this checkout.
pub fn relative_base(base: &Path) -> PathBuf {
    let base = std::path::absolute(base).unwrap();
    let current = std::env::current_dir().unwrap();
    let target = base.components().collect::<Vec<_>>();
    let origin = current.components().collect::<Vec<_>>();
    let shared = target
        .iter()
        .zip(&origin)
        .take_while(|(a, b)| a == b)
        .count();
    if shared == 0 {
        return base;
    }
    let mut relative = PathBuf::new();
    for _ in &origin[shared..] {
        relative.push("..");
    }
    for component in &target[shared..] {
        relative.push(component.as_os_str());
    }
    relative
}

/// A content mod folder from `(path, text)` pairs, read as the game reads it.
pub fn folder_mod(dir: &Path, files: &[(&str, &str)]) -> ContentMod {
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = sunrise_mod_package::Package::read(dir).unwrap();
    ContentMod {
        id: package.manifest.id.clone(),
        content: Content::read(dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    }
}

pub fn manifest(id: &str, namespaces: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\nnamespaces = [{namespaces}]\n"
    )
}

/// A base root of one template with two states, and a mod that adds a block
/// to it. `declarations` go in as the block file.
pub fn block_base(tag: &str, declarations: &[&str]) -> (PathBuf, Vec<ContentMod>) {
    block_base_of(
        tag,
        declarations,
        r#"{"states":[
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"x"}}],
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"y"}}]],
      "defaults":{"minecraft:test":0}}"#,
    )
}

/// The same over a base root whose `facts.json` is `facts`: the other tables
/// the plan reads are built from its state count, so a template can have one
/// state and no properties of its own (a `stone`) as easily as two.
pub fn block_base_of(tag: &str, declarations: &[&str], facts: &str) -> (PathBuf, Vec<ContentMod>) {
    let scratch = std::env::temp_dir().join(format!("sunrise-mod-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("facts.json"), facts).unwrap();
    let count = match serde_json::from_str::<J>(facts).unwrap().get("states") {
        Some(J::Arr(rows)) => rows.len(),
        _ => panic!("{tag}: facts.json has no states"),
    };
    let rows = vec![r#"{"item":"minecraft:test","sound":{"identity":"other"}}"#; count].join(",");
    fs::write(
        base.join("gameplay-facts.json"),
        format!(r#"{{"states":[{rows}]}}"#),
    )
    .unwrap();
    fs::write(base.join("items.json"), r#"{"minecraft:test":{}}"#).unwrap();
    fs::write(
        base.join("registries.json"),
        r#"{"minecraft:item":{"entries":{"minecraft:test":{"protocol_id":0}}},"minecraft:block":{"entries":{"minecraft:test":{"protocol_id":0}}}}"#,
    )
    .unwrap();
    let mods: Vec<ContentMod> = declarations
        .iter()
        .enumerate()
        .map(|(i, declaration)| {
            let dir = scratch.join(format!("mod{i}"));
            folder_mod(
                &dir,
                &[
                    ("sunrise-mod.toml", &manifest("testmod", "")),
                    ("data/testmod/sunrise/block/test.json", *declaration),
                ],
            )
        })
        .collect();
    (base, mods)
}

/// A base root of one block with three biomes, two biome tags (one nesting
/// the other), three vanilla placed features and entity types: what biome
/// modifiers read.
pub fn biome_base(tag: &str) -> PathBuf {
    let (base, _) = block_base(tag, &[]);
    let worldgen = base.join("worldgen");
    fs::create_dir_all(&worldgen).unwrap();
    let bundle = |entries: &[(&str, &str)]| -> String {
        let map: BTreeMap<&str, &str> = entries.iter().copied().collect();
        serde_json::to_string(&map).unwrap()
    };
    let files = [
        (
            "biomes.json",
            bundle(&[
                (
                    "minecraft:plains",
                    r#"{"temperature":0.8,"downfall":0.4,"features":[[],["minecraft:lake"],[],[],[],[],["minecraft:ore_coal"]],"attributes":{"minecraft:gameplay/natural_mob_spawns":{"argument":{"spawn_costs":{"minecraft:skeleton":{"energy_budget":0.7,"charge":0.15}},"spawns_by_category":{"monster":[{"type":"minecraft:zombie","count":4,"weight":95},{"type":"minecraft:skeleton","count":4,"weight":100}]}},"modifier":"overlay"}}}"#,
                ),
                (
                    "minecraft:desert",
                    r#"{"temperature":2.0,"downfall":0.0,"features":[["minecraft:fossil"]]}"#,
                ),
                (
                    "minecraft:nether_wastes",
                    r#"{"temperature":2.0,"downfall":0.0,"features":[[],[],[],[],[],[],["minecraft:ore_coal"]]}"#,
                ),
            ]),
        ),
        (
            "biome_tags.json",
            bundle(&[
                (
                    "minecraft:is_overworld",
                    r##"{"values":["#minecraft:is_plains_like","minecraft:desert",{"id":"othermod:x","required":false}]}"##,
                ),
                (
                    "minecraft:is_plains_like",
                    r#"{"values":["minecraft:plains"]}"#,
                ),
            ]),
        ),
        (
            "placed_features.json",
            bundle(&[
                ("minecraft:ore_coal", "{}"),
                ("minecraft:lake", "{}"),
                ("minecraft:fossil", "{}"),
            ]),
        ),
        ("features.json", bundle(&[])),
    ];
    for (name, text) in files {
        fs::write(worldgen.join(name), text).unwrap();
    }
    fs::write(
        base.join("entity-types.json"),
        r#"{"minecraft:zombie":{"category":"monster"},"minecraft:skeleton":{"category":"monster"},"minecraft:stray":{"category":"monster"},"minecraft:rabbit":{"category":"creature"}}"#,
    )
    .unwrap();
    let entity_tags = base.join("data/minecraft/tags/entity_type");
    fs::create_dir_all(&entity_tags).unwrap();
    fs::write(
        entity_tags.join("skeletons.json"),
        r#"{"values":["minecraft:skeleton","minecraft:stray",{"id":"othermod:bone","required":false}]}"#,
    )
    .unwrap();
    base
}

/// A mod adding a zinc ore and its biome modifiers (`(file name, text)`).
pub fn zinc_mod(dir: &Path, modifiers: &[(&str, &str)]) -> ContentMod {
    let mut files = vec![
        ("sunrise-mod.toml".to_owned(), manifest("zinc", "")),
        (
            "data/zinc/worldgen/feature/ore_zinc.json".to_owned(),
            r#"{"type":"minecraft:ore","config":{}}"#.to_owned(),
        ),
        (
            "data/zinc/worldgen/placed_feature/ore_zinc.json".to_owned(),
            r#"{"feature":"zinc:ore_zinc","placement":[]}"#.to_owned(),
        ),
    ];
    for (name, text) in modifiers {
        files.push((
            format!("data/zinc/neoforge/biome_modifier/{name}.json"),
            (*text).to_owned(),
        ));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    folder_mod(dir, &files)
}

/// A small stand-in for the base root: nested folders, an empty file, an empty folder.
pub fn small_base(base: &Path) {
    for (name, bytes) in [
        ("items.json", &b"{}"[..]),
        ("empty.log", &b""[..]),
        ("assets/minecraft/lang/en_us.json", &b"{\"a\": 1}"[..]),
        (
            "data/minecraft/tags/block/logs.json",
            &b"{\"values\": []}"[..],
        ),
    ] {
        let path = base.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::create_dir_all(base.join("worldgen/empty-folder")).unwrap();
}

// These fixtures accept every listed content package on both sides. Production
// captures the inventory after admission, including active code-only packages.
pub fn fixture_inventory(mods: &[ContentMod]) -> sunrise_mod_package::enabled::EnabledMods {
    sunrise_mod_package::enabled::EnabledMods::new(
        mods.iter()
            .map(|package| (package.id.clone(), mod_api::Side::Both)),
    )
}

pub fn build_fixture(
    base: &Path,
    jars: &Path,
    mods: &[ContentMod],
    out: &Path,
) -> Result<Vec<String>> {
    super::build(base, jars, mods, out, &fixture_inventory(mods))
}

pub fn resource_root_fixture(
    base: &Path,
    jars: &Path,
    mods: &[ContentMod],
    cache: &Path,
) -> Result<PathBuf> {
    super::resource_root(base, jars, mods, cache, &fixture_inventory(mods))
}

pub fn content_files(
    resources: &sunrise_assets::resource_source::ResourceRoot,
) -> Vec<(String, Vec<Vec<u8>>)> {
    let content = crate::datapack::DataPack::from_source(
        "sunrise-content".into(),
        resources.data_changes().clone(),
    );
    crate::datapack::DataStore::load(&crate::datapack::DataPacks::new(&[&content]))
        .files
        .into_iter()
        .collect()
}

/// A content mod folder: a full block, a door and an item-less glass from
/// templates, items, a recipe, a vanilla tag addition and a language file.
pub fn ruby_mod(dir: &Path) -> ContentMod {
    ruby_mod_with_pistons(dir, false)
}

pub fn ruby_mod_with_pistons(dir: &Path, pistons: bool) -> ContentMod {
    let files: [(&str, &str); 11] = [
        (
            "sunrise-mod.toml",
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/ruby/sunrise/block/ruby_block.json",
            r##"{"template": "minecraft:iron_block", "properties": {"hardness": 4.0, "light": 3,
                "sounds": "minecraft:amethyst_block", "map_color": 28, "friction": 0.8,
                "random_ticks": true, "occludes": false, "destroy_overlay": false},
                "components": {"minecraft:max_stack_size": 16,
                    "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}}"##,
        ),
        (
            "data/ruby/sunrise/block/ruby_door.json",
            r#"{"template": "minecraft:oak_door", "properties": {"push_reaction": "IMMOVEABLE"}}"#,
        ),
        (
            "data/ruby/sunrise/block/ruby_glass.json",
            r#"{"template": "minecraft:glass", "item": false,
                "properties": {"render_shape": "ENTITYBLOCK_ANIMATED"}}"#,
        ),
        (
            "data/ruby/sunrise/item/ruby.json",
            r#"{"template": "minecraft:diamond", "max_stack_size": 16, "rarity": "rare", "block": "ruby:ruby_glass"}"#,
        ),
        (
            "data/ruby/sunrise/item/ruby_sword.json",
            r#"{"template": "minecraft:iron_sword", "block": "minecraft:wheat", "components": {"minecraft:max_damage": 900,
                "minecraft:attribute_modifiers": [{"type": "minecraft:attack_damage",
                    "id": "minecraft:base_attack_damage", "amount": 7.0,
                    "operation": "add_value", "slot": "mainhand"}],
                "!minecraft:repairable": {}}}"#,
        ),
        (
            "data/ruby/sunrise/item/ruby_stew.json",
            r#"{"template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl"}"#,
        ),
        (
            "data/ruby/recipe/ruby_from_stew.json",
            r#"{"type": "minecraft:crafting_shapeless", "category": "misc",
                "ingredients": ["ruby:ruby_stew"], "result": {"id": "ruby:ruby"}}"#,
        ),
        // Carried-over tags name blocks left out and other mods' ids too.
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            r##"{"values": ["ruby:ruby_block", "ruby:left_out", "othermod:thing",
                {"id": "othermod:optional", "required": false}, "#c:storage_blocks/ruby"]}"##,
        ),
        (
            "data/c/tags/item/gems/ruby.json",
            r#"{"values": ["ruby:ruby", "othermod:ruby"]}"#,
        ),
        (
            "assets/ruby/lang/en_us.json",
            r#"{"block.ruby.ruby_block": "Block of Ruby"}"#,
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    if pistons {
        for (name, text) in [
            ("z_stone", r#"{"template":"minecraft:stone"}"#),
            (
                "z_stuck",
                r#"{"template":"minecraft:stone","properties":{"push_reaction":"IMMOVEABLE"}}"#,
            ),
        ] {
            fs::write(
                dir.join(format!("data/ruby/sunrise/block/{name}.json")),
                text,
            )
            .unwrap();
        }
    }
    let package = sunrise_mod_package::Package::read(dir).unwrap();
    ContentMod {
        id: "ruby".into(),
        content: Content::read(dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    }
}
