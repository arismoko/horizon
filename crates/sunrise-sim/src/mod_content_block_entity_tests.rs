use super::*;

fn base(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "sunrise-block-entity-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("assets/minecraft/blockstates")).unwrap();
    for (file, text) in [
        (
            "facts.json",
            r#"{"states":[["minecraft:stone"]],"defaults":{"minecraft:stone":0}}"#,
        ),
        (
            "gameplay-facts.json",
            r#"{"states":[{"item":"minecraft:stone"}]}"#,
        ),
        (
            "registries.json",
            r#"{"minecraft:item":{"entries":{}},"minecraft:block":{"entries":{}},
                "minecraft:block_entity_type":{"entries":{"minecraft:sign":{"protocol_id":8}}}}"#,
        ),
        ("items.json", "{}"),
        ("assets/minecraft/blockstates/stone.json", "{}"),
        ("sunrise-resources.json", "{}"),
    ] {
        fs::write(base.join(file), text).unwrap();
    }
    base
}

fn kind(id: &str, blocks: &[&str]) -> BlockEntityType {
    BlockEntityType {
        id: id.into(),
        blocks: blocks.iter().map(|b| (*b).into()).collect(),
    }
}

fn declared_mod() -> ContentMod {
    let mut content = Content::declared("test");
    content
        .declare(
            &["test:a", "test:b"].map(|id| Block {
                id: id.into(),
                template: "minecraft:stone".into(),
                ..Block::default()
            }),
            &[],
        )
        .unwrap();
    ContentMod {
        id: "test".into(),
        content,
        jars: Vec::new(),
    }
}

#[test]
fn code_types_join_package_and_code_blocks_before_planning() {
    let base = base("merge");
    let mut mods = vec![declared_mod()];
    let blocks = [Block {
        id: "minecraft:custom_block".into(),
        template: "minecraft:stone".into(),
        ..Block::default()
    }];
    let batches = BTreeMap::from([(
        "test",
        DeclaredContent {
            blocks: blocks
                .into_iter()
                .map(|b| b.validate_local().unwrap())
                .collect(),
            types: vec![kind(
                "minecraft:custom_type",
                &["test:b", "minecraft:custom_block"],
            )],
            ..Default::default()
        },
    )]);
    let mut errors = Vec::new();
    accept_declared(&mut mods, &batches, &[], &BTreeSet::new(), &mut errors).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let plan = Plan::new(&base, &mods).unwrap();
    assert_eq!(plan.block_entity_types.len(), 1);
    assert_eq!(
        plan.block_entity_factories.get("test:b"),
        Some(&"minecraft:custom_type")
    );
    assert_eq!(
        plan.block_entity_factories.get("minecraft:custom_block"),
        Some(&"minecraft:custom_type")
    );
    assert!(!plan.block_entity_factories.contains_key("test:a"));
    drop(plan);
    // A type-only code namespace is not skipped by the content merge.
    let type_only = BTreeMap::from([(
        "other",
        DeclaredContent {
            types: vec![kind("other:empty", &[])],
            ..Default::default()
        },
    )]);
    accept_declared(&mut mods, &type_only, &[], &BTreeSet::new(), &mut errors).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(mods[1].content.block_entity_types[0].id, "other:empty");
    assert_eq!(Plan::new(&base, &mods).unwrap().block_entity_types.len(), 2);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn cross_namespace_declarations_do_not_replace_existing_block_or_item_keys() {
    let base = base("duplicate-keys");
    fs::write(base.join("items.json"), r#"{"minecraft:stone":{}}"#).unwrap();
    let block = Block {
        id: "test:a".into(),
        template: "minecraft:stone".into(),
        ..Block::default()
    };
    let item = Item {
        id: "test:b".into(),
        template: "minecraft:stone".into(),
        ..Item::default()
    };
    let conflict = |kind, id: &str, reason: &str| {
        DeclarationRefusal::new(
            "other",
            kind,
            DeclarationError::Rejected {
                id: id.into(),
                reason: reason.into(),
            },
        )
    };
    for (batch, expected) in [
        (
            DeclaredContent {
                blocks: vec![block.validate_local().unwrap()],
                ..Default::default()
            },
            Ok(conflict(
                DeclarationKind::Block,
                "test:a",
                "declared twice (already declared by mod test)",
            )),
        ),
        (
            DeclaredContent {
                items: vec![item.clone().validate_local().unwrap()],
                ..Default::default()
            },
            Ok(conflict(
                DeclarationKind::Item,
                "test:b",
                "declared twice, or is also a block's item (already declared by mod test)",
            )),
        ),
        (
            DeclaredContent {
                items: vec![
                    Item {
                        id: "minecraft:stone".into(),
                        ..item
                    }
                    .validate_local()
                    .unwrap(),
                ],
                ..Default::default()
            },
            Err("other: minecraft:stone is already an item"),
        ),
    ] {
        let mut mods = vec![declared_mod()];
        mods[0]
            .content
            .blocks
            .iter_mut()
            .find(|block| block.id == "test:b")
            .unwrap()
            .item = true;
        let mut refusals = Vec::new();
        accept_declared(
            &mut mods,
            &BTreeMap::from([("other", batch)]),
            &[],
            &BTreeSet::new(),
            &mut refusals,
        )
        .unwrap();
        match expected {
            Ok(refusal) => {
                // A second package's duplicate is caught while joining the batch.
                assert_eq!(refusals, [refusal]);
                assert_eq!(mods.len(), 1);
                assert_eq!(mods[0].id, "test");
            }
            Err(expected) => {
                // The prepared base item registry is first consulted by the plan.
                assert!(refusals.is_empty(), "{refusals:?}");
                assert_eq!(Plan::new(&base, &mods).err().unwrap().to_string(), expected);
            }
        }
    }
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn validates_declared_type_ids_and_exclusive_known_mod_block_membership() {
    let base = base("validation");
    let mut mods = [declared_mod()];
    for (types, error) in [
        (
            vec![
                kind("test:machine", &["test:a"]),
                kind("test:machine", &["test:b"]),
            ],
            "block entity type test:machine is already declared",
        ),
        (
            vec![kind("minecraft:sign", &["test:a"])],
            "block entity type minecraft:sign is already declared",
        ),
        (
            vec![kind("test:machine", &["test:a", "test:a"])],
            "block test:a belongs to both block entity types test:machine and test:machine",
        ),
        (
            vec![kind("test:one", &["test:a"]), kind("test:two", &["test:a"])],
            "block test:a belongs to both block entity types test:one and test:two",
        ),
        (
            vec![kind("test:machine", &["missing:block"])],
            "block entity type test:machine: missing:block is not a block",
        ),
        (
            vec![kind("test:machine", &["minecraft:stone"])],
            "mod test: block entity type test:machine cannot own vanilla block minecraft:stone",
        ),
    ] {
        mods[0].content.block_entity_types = types;
        assert_eq!(Plan::new(&base, &mods).err().unwrap().to_string(), error);
    }
    mods[0].content.block_entity_types = vec![kind("test:machine", &["test:b", "test:a"])];
    let plan = Plan::new(&base, &mods).unwrap();
    assert_eq!(
        serde_json::to_string(&plan.block_entity_factories).unwrap(),
        r#"{"test:a":"test:machine","test:b":"test:machine"}"#
    );
    drop(plan);
    mods[0].content.block_entity_types.clear();
    let plan = Plan::new(&base, &mods).unwrap();
    let mut registry = read_json(&base.join("registries.json")).unwrap();
    plan.json("registries.json", &mut registry).unwrap();
    let registry: serde_json::Value = serde_json::to_value(registry).unwrap();
    assert_eq!(
        registry["minecraft:block_entity_type"]["entries"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn code_declared_types_build_a_root_and_membership_changes_invalidate_its_cache() {
    let base = base("cache");
    let mut mods = [ContentMod {
        id: "test".into(),
        content: Content::declared("test"),
        jars: Vec::new(),
    }];
    let cache = base.with_extension("cache");
    // Even before a code-only mod declares content, its accepted package ID
    // belongs to a distinct resource generation for load conditions.
    let code_only = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(code_only, base);
    assert_eq!(
        sunrise_mod_package::enabled::EnabledMods::read(&code_only)
            .unwrap()
            .server_ids()
            .collect::<Vec<_>>(),
        vec!["test"]
    );
    assert_eq!(
        fs::read(code_only.join("registries.json")).unwrap(),
        fs::read(base.join("registries.json")).unwrap()
    );
    mods[0].content.block_entity_types = vec![kind("test:machine", &[])];
    let root = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(root, base);
    assert_ne!(root, code_only);
    assert!(root.join(crate::content_identity::MARKER).exists());
    assert_eq!(
        fs::read_to_string(root.join(BLOCK_ENTITY_TYPES)).unwrap(),
        "{}"
    );
    mods[0].content.blocks = declared_mod().content.blocks;
    mods[0].content.block_entity_types[0].blocks = vec!["test:a".into()];
    let with_a = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(with_a, root);
    assert_eq!(
        fs::read_to_string(root.join(BLOCK_ENTITY_TYPES)).unwrap(),
        "{}"
    );
    let stamp = fs::read(with_a.join(STAMP)).unwrap();
    assert_eq!(
        fs::read_to_string(with_a.join(BLOCK_ENTITY_TYPES)).unwrap(),
        r#"{"test:a":"test:machine"}"#
    );
    // Same type and wire ID, different valid blocks: the overlay must not be reused.
    mods[0].content.block_entity_types[0].blocks = vec!["test:b".into()];
    let with_b = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(with_a, with_b);
    assert_eq!(
        resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap(),
        with_b
    );
    assert_eq!(fs::read(with_a.join(STAMP)).unwrap(), stamp);
    assert_eq!(
        fs::read_to_string(with_b.join(BLOCK_ENTITY_TYPES)).unwrap(),
        r#"{"test:b":"test:machine"}"#
    );
    let registry =
        sunrise_protocol::registry::Builtin::load(&with_b.join("registries.json")).unwrap();
    assert_eq!(
        registry
            .get("minecraft:block_entity_type")
            .unwrap()
            .id("test:machine"),
        Some(9)
    );
    fs::remove_dir_all(cache).unwrap();
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn code_items_can_reference_both_owned_namespaces_in_one_batch() {
    let manifest = sunrise_mod_package::Manifest::parse(
        "id='porter'\nname='Porter'\nversion='1'\napi='0.2'\ncapabilities=['registries']\nnamespaces=['legacy']"
    ).unwrap();
    let mut mods = Vec::new();
    let items = [
        ("legacy:wrench", "porter:spoon"),
        ("porter:spoon", "legacy:wrench"),
    ]
    .map(|(id, remainder)| sunrise_mod_package::content::Item {
        id: id.into(),
        template: "minecraft:stick".into(),
        crafting_remainder: Some(remainder.into()),
        ..sunrise_mod_package::content::Item::default()
    });
    let mut batches = BTreeMap::from([(
        "porter",
        DeclaredContent {
            items: items
                .iter()
                .cloned()
                .map(|i| i.validate_local().unwrap())
                .collect(),
            ..Default::default()
        },
    )]);
    let mut errors = Vec::new();
    accept_declared(
        &mut mods,
        &batches,
        &[&manifest],
        &BTreeSet::new(),
        &mut errors,
    )
    .unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].id, "porter");
    assert_eq!(mods[0].content.namespaces, ["porter", "legacy"]);
    assert_eq!(mods[0].content.items, items);

    // No file row to infer an owner from, and neither namespace installs on refusal.
    let mut broken = items[0].clone();
    broken
        .components
        .insert("legacy:unknown".into(), serde_json::json!(1));
    batches.get_mut("porter").unwrap().items[0] = broken.validate_local().unwrap();
    mods.clear();
    accept_declared(
        &mut mods,
        &batches,
        &[&manifest],
        &BTreeSet::new(),
        &mut errors,
    )
    .unwrap();
    assert!(mods.is_empty());
    assert!(matches!(errors.as_slice(), [refusal]
        if refusal.mod_id == "porter" && refusal.kind == DeclarationKind::Item
        && matches!(&refusal.error, DeclarationError::Rejected { id, reason }
            if id == "legacy:wrench" && reason.contains("legacy:unknown"))));
}

#[test]
fn code_declarations_join_the_manifest_owner_of_a_secondary_namespace() {
    let base = base("owned-namespace");
    let mut content = Content::declared("porter");
    content.namespaces.push("legacy".into());
    let mut mods = vec![ContentMod {
        id: "porter".into(),
        content,
        jars: vec![],
    }];
    let blocks = [Block {
        id: "legacy:pot".into(),
        template: "minecraft:stone".into(),
        ..Block::default()
    }];
    let batches = BTreeMap::from([(
        "porter",
        DeclaredContent {
            blocks: blocks
                .into_iter()
                .map(|b| b.validate_local().unwrap())
                .collect(),
            types: vec![kind("legacy:pot_type", &["legacy:pot"])],
            ..Default::default()
        },
    )]);
    let mut errors = Vec::new();
    accept_declared(&mut mods, &batches, &[], &BTreeSet::new(), &mut errors).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        mods.len(),
        1,
        "the secondary namespace is not a synthetic second package"
    );
    assert_eq!(mods[0].id, "porter");
    assert_eq!(mods[0].content.blocks[0].id, "legacy:pot");
    let plan = Plan::new(&base, &mods).unwrap();
    assert_eq!(
        plan.block_entity_factories.get("legacy:pot"),
        Some(&"legacy:pot_type")
    );
    drop(plan);
    fs::remove_dir_all(base).unwrap();
}

/// Declared boxes are in block units, one block being 1: a code-declared half slab resolves to
/// the bounds `[0, 0, 0, 1, 0.5, 1]`, as the same box in a JSON declaration does.
#[test]
fn a_code_declared_half_slab_resolves_to_block_unit_bounds() {
    let (base, _) = block_base("code-slab", &[]);
    let mut content = Content::declared("testmod");
    content
        .declare(
            &[Block {
                id: "testmod:slab".into(),
                template: "minecraft:test".into(),
                item: false,
                collision_shapes: Some(BlockShapes(StateValues {
                    property: String::new(),
                    properties: Vec::new(),
                    values: BTreeMap::new(),
                    default: Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
                })),
                ..Block::default()
            }],
            &[],
        )
        .unwrap();
    let mods = vec![ContentMod {
        id: "testmod".into(),
        content,
        jars: Vec::new(),
    }];
    let plan = Plan::new(&base, &mods).unwrap();
    for shape in plan.blocks[0].collision_shapes.as_ref().unwrap() {
        let bounds: Vec<[f64; 6]> = shape
            .boxes()
            .iter()
            .map(|b| {
                [
                    b.min(Axis::X),
                    b.min(Axis::Y),
                    b.min(Axis::Z),
                    b.max(Axis::X),
                    b.max(Axis::Y),
                    b.max(Axis::Z),
                ]
            })
            .collect();
        assert_eq!(bounds, vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]);
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn shapes_from_block_entity_is_refused_by_name_without_a_block_entity_type() {
    let base = base("published-shapes");
    let mut mods = [declared_mod()];
    mods[0]
        .content
        .blocks
        .iter_mut()
        .find(|block| block.id == "test:a")
        .unwrap()
        .shapes_from_block_entity = true;
    assert_eq!(
        Plan::new(&base, &mods).err().unwrap().to_string(),
        "test:a: shapes_from_block_entity needs a block entity type that owns the block"
    );
    // A type that owns another block does not do.
    mods[0].content.block_entity_types = vec![kind("test:machine", &["test:b"])];
    assert!(Plan::new(&base, &mods).is_err());
    mods[0].content.block_entity_types = vec![kind("test:machine", &["test:a"])];
    let plan = Plan::new(&base, &mods).unwrap();
    // Only the flagged block's states reach the collision and support tables' scan metadata.
    let mut table = Tag::Compound(nbt::Compound::new());
    plan.published_states(&mut table).unwrap();
    let flagged = plan
        .blocks
        .iter()
        .find(|block| block.decl.id == "test:a")
        .unwrap();
    let expected: Vec<i32> = (0..flagged.order.len())
        .map(|offset| (flagged.rename.first as usize + offset) as i32)
        .collect();
    assert_eq!(
        table.get("published_shapes"),
        Some(&Tag::IntArray(expected))
    );
    drop(plan);
    mods[0]
        .content
        .blocks
        .iter_mut()
        .for_each(|block| block.shapes_from_block_entity = false);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut table = Tag::Compound(nbt::Compound::new());
    plan.published_states(&mut table).unwrap();
    assert!(table.get("published_shapes").is_none());
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn scaffolding_context_collision_lists_its_states_and_needs_scaffolding_properties() {
    let base = base("scaffolding-collision");
    let mut mods = [declared_mod()];
    fn declared(mods: &mut [ContentMod; 1]) -> &mut Block {
        mods[0]
            .content
            .blocks
            .iter_mut()
            .find(|block| block.id == "test:a")
            .unwrap()
    }
    declared(&mut mods).context_collision = Some(ContextCollision::Scaffolding);
    // Stone's one state has neither of scaffolding's properties.
    assert_eq!(
        Plan::new(&base, &mods).err().unwrap().to_string(),
        "test:a: context_collision scaffolding: invalid scaffold distance"
    );
    declared(&mut mods).state_properties = BTreeMap::from([
        ("bottom".to_owned(), StateProperty::Boolean),
        (
            "distance".to_owned(),
            StateProperty::Enum {
                values: (0..8).map(|n| n.to_string()).collect(),
            },
        ),
    ]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut table = Tag::Compound(nbt::Compound::new());
    plan.scaffolding_states(&mut table).unwrap();
    let block = plan
        .blocks
        .iter()
        .find(|block| block.decl.id == "test:a")
        .unwrap();
    let expected: Vec<i32> = (0..16)
        .map(|offset| block.rename.first as i32 + offset)
        .collect();
    assert_eq!(block.order.len(), 16);
    assert_eq!(
        table.get("scaffolding_collision"),
        Some(&Tag::IntArray(expected))
    );
    fs::remove_dir_all(base).unwrap();
}
