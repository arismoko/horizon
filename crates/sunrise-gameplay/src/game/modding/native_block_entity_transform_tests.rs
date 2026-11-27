//! Placement exercises the real processor and the game-side registration bridge.
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::{Compound, Limits, NamedTag, Tag, write_network};
use sunrise_gameplay_common::game::native_block_entity_transform::prepare;
use sunrise_world::{
    compression::Compression,
    region::ChunkRecord,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        structure::{
            Mirror, Rotation, StateTransforms, StructureResources, StructureTemplate,
            processor::{PlaceSettings, place_in_world},
        },
    },
};

type Calls = Arc<Mutex<Vec<mod_api::StructureTransform>>>;
struct Registration(Calls);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Registration {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let calls = self.0.clone();
        r.block_entity_type("machine")
            .transform(
                move |data: &mut mod_api::Compound, t: mod_api::StructureTransform| {
                    calls.lock().unwrap().push(t);
                    let facing = match data.get(&"facing".into()) {
                        Some(Tag::String(s)) => s.to_string().unwrap(),
                        _ => return Err(mod_api::TransformRefusal::new("missing facing")),
                    };
                    let facing = match (t.mirror, facing.as_str()) {
                        (mod_api::StructureMirror::LeftRight, "north") => "south",
                        (mod_api::StructureMirror::LeftRight, "south") => "north",
                        (mod_api::StructureMirror::FrontBack, "east") => "west",
                        (mod_api::StructureMirror::FrontBack, "west") => "east",
                        (_, f) => f,
                    };
                    let directions = ["north", "east", "south", "west"];
                    let index = directions.iter().position(|f| *f == facing).unwrap();
                    let turns = match t.rotation {
                        mod_api::StructureRotation::None => 0,
                        mod_api::StructureRotation::Clockwise90 => 1,
                        mod_api::StructureRotation::Clockwise180 => 2,
                        mod_api::StructureRotation::Counterclockwise90 => 3,
                        _ => unreachable!(),
                    };
                    data.insert(
                        "facing".into(),
                        Tag::String(directions[(index + turns) % 4].into()),
                    );
                    Ok(())
                },
            )
            .build();
        r.block_entity_type("hostile")
            .transform(
                |data: &mut mod_api::Compound, _: mod_api::StructureTransform| {
                    data.clear();
                    data.insert("id".into(), Tag::String("minecraft:chest".into()));
                    data.insert("x".into(), Tag::Int(-1));
                    data.insert("facing".into(), Tag::String("east".into()));
                    Ok(())
                },
            )
            .build();
        r.block_entity_type("plain").build();
        r.block_entity_type("refuses")
            .transform(
                |data: &mut mod_api::Compound, _: mod_api::StructureTransform| {
                    data.insert("visited".into(), Tag::Byte(1));
                    Err(mod_api::TransformRefusal::new("fixture refusal"))
                },
            )
            .build();
        r.block_entity_type("deep")
            .transform(
                |data: &mut mod_api::Compound, _: mod_api::StructureTransform| {
                    let mut tag = Tag::Byte(1);
                    // Deeper than vanilla NbtAccounter.MAX_STACK_DEPTH (512).
                    for _ in 0..514 {
                        tag = Tag::Compound([("child".into(), tag)].into());
                    }
                    data.insert("nested".into(), tag);
                    Ok(())
                },
            )
            .build();
        r.block_entity_type("panics")
            .transform(
                |data: &mut mod_api::Compound,
                 _: mod_api::StructureTransform|
                 -> Result<(), mod_api::TransformRefusal> {
                    data.insert("visited".into(), Tag::Byte(1));
                    panic!("fixture panic")
                },
            )
            .build();
    }
}
struct World {
    pub state: BlockStateId,
    stored: Option<Compound>,
}
impl FeatureWorld for World {
    type Error = PlacementQueryError;
    fn block(&mut self, _: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        Ok(self.state)
    }
    fn set_block(&mut self, _: [i32; 3], state: BlockStateId, _: i32) -> Result<bool, Self::Error> {
        self.state = state;
        Ok(true)
    }
}
impl PlacementContext for World {
    fn set_block_entity(&mut self, _: [i32; 3], data: Compound) -> Result<(), PlacementQueryError> {
        assert_ne!(self.state, BlockStateId(1));
        self.stored = Some(data);
        Ok(())
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(0, 256, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn height(
        &mut self,
        _: sunrise_world::heightmap::HeightmapType,
        _: i32,
        _: i32,
    ) -> Result<i32, PlacementQueryError> {
        Ok(64)
    }
}
const NAMES: [&str; 6] = [
    "minecraft:air",
    "minecraft:barrier",
    "test:north",
    "test:east",
    "test:south",
    "test:west",
];
fn resources(dir: &std::path::Path) -> StructureResources {
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(sunrise_core::version::DATA_VERSION)),
        (
            "states",
            Tag::List(
                NAMES
                    .iter()
                    .map(|n| Tag::compound([("id", Tag::String((*n).into()))]))
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::compound(NAMES.iter().zip(0..).map(|(n, i)| (*n, Tag::Int(i)))),
        ),
    ]))
    .unwrap();
    // Hand-written vanilla north/east/south/west transforms; barrier and air stay fixed.
    let transforms = StateTransforms::from_json(
        r#"{
        "rotate":[[0,0,0],[1,1,1],[3,4,5],[4,5,2],[5,2,3],[2,3,4]],
        "mirror":[[0,0],[1,1],[4,2],[3,5],[2,4],[5,3]],
        "full_block":[false,true,false,false,false,false],
        "loot_containers":[],"block_entities":{}}
    "#,
        6,
    )
    .unwrap();
    StructureResources::new(
        Box::leak(Box::new(registry)),
        transforms,
        dir,
        Default::default(),
        &Default::default(),
    )
}
fn place(
    resources: &StructureResources,
    id: &str,
    facing: &str,
    rotation: Rotation,
    mirror: Mirror,
) -> (World, Compound) {
    place_block(
        resources,
        id,
        facing,
        &format!("test:{facing}"),
        rotation,
        mirror,
    )
}
fn place_block(
    resources: &StructureResources,
    id: &str,
    facing: &str,
    block: &str,
    rotation: Rotation,
    mirror: Mirror,
) -> (World, Compound) {
    let data: Compound = [
        ("id".into(), Tag::String(id.into())),
        ("facing".into(), Tag::String(facing.into())),
        ("x".into(), Tag::Int(3)),
        ("y".into(), Tag::Int(64)),
        ("z".into(), Tag::Int(5)),
        ("bits".into(), Tag::Float(0x7fc00001)),
        (
            "utf16".into(),
            Tag::String(sunrise_core::nbt::JavaString(vec![0xd800])),
        ),
    ]
    .into();
    let template = template(resources, block, &data);
    (place_template(resources, &template, rotation, mirror), data)
}

fn template(resources: &StructureResources, block: &str, data: &Compound) -> StructureTemplate {
    let tag = Tag::compound([
        ("size", Tag::List(vec![Tag::Int(1); 3])),
        (
            "palette",
            Tag::List(vec![Tag::compound([("Name", Tag::String(block.into()))])]),
        ),
        (
            "blocks",
            Tag::List(vec![Tag::compound([
                ("pos", Tag::List(vec![Tag::Int(0); 3])),
                ("state", Tag::Int(0)),
                ("nbt", Tag::Compound(data.clone())),
            ])]),
        ),
    ]);
    StructureTemplate::from_nbt(&tag, resources.registry, &resources.transforms)
}

fn place_template(
    resources: &StructureResources,
    template: &StructureTemplate,
    rotation: Rotation,
    mirror: Mirror,
) -> World {
    let mut world = World {
        state: BlockStateId(0),
        stored: None,
    };
    let mut settings = PlaceSettings::new(rotation);
    settings.mirror = mirror;
    settings.known_shape = true;
    settings.waterlogging = false;
    assert!(
        place_in_world(
            &mut world,
            resources,
            template,
            [3, 64, 5],
            [3, 64, 5],
            &mut settings,
            None,
            2
        )
        .unwrap()
    );
    world
}
struct Capture(Mutex<Vec<String>>);
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(record.args().to_string());
    }
    fn flush(&self) {}
}
static LOG: Capture = Capture(Mutex::new(Vec::new()));

#[test]
#[ignore = "daily"]
fn structure_transform_placement_rotations_mirrors_unset_and_refusal() {
    // The logger is process-global; isolate this assertion from other log-capture tests.
    if std::env::var_os("SUNRISE_STRUCTURE_TRANSFORM_LOG_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "game::native_block_entity_transform_tests::structure_transform_placement_rotations_mirrors_unset_and_refusal", "--nocapture"])
            .env("SUNRISE_STRUCTURE_TRANSFORM_LOG_CHILD", "1")
            .status().unwrap();
        assert!(status.success());
        return;
    }
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    let calls = Calls::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resources = resources(dir.path());
        prepare(resources.registry);
        for (rotation, mirror, input, expected, api_rotation, api_mirror) in [
            (
                Rotation::None,
                Mirror::None,
                "north",
                "north",
                mod_api::StructureRotation::None,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::None,
                Mirror::LeftRight,
                "north",
                "south",
                mod_api::StructureRotation::None,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::None,
                Mirror::FrontBack,
                "north",
                "north",
                mod_api::StructureRotation::None,
                mod_api::StructureMirror::FrontBack,
            ),
            (
                Rotation::Clockwise90,
                Mirror::None,
                "north",
                "east",
                mod_api::StructureRotation::Clockwise90,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::Clockwise90,
                Mirror::LeftRight,
                "north",
                "west",
                mod_api::StructureRotation::Clockwise90,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::Clockwise90,
                Mirror::FrontBack,
                "north",
                "east",
                mod_api::StructureRotation::Clockwise90,
                mod_api::StructureMirror::FrontBack,
            ),
            (
                Rotation::Clockwise180,
                Mirror::None,
                "north",
                "south",
                mod_api::StructureRotation::Clockwise180,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::Clockwise180,
                Mirror::LeftRight,
                "north",
                "north",
                mod_api::StructureRotation::Clockwise180,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::Clockwise180,
                Mirror::FrontBack,
                "north",
                "south",
                mod_api::StructureRotation::Clockwise180,
                mod_api::StructureMirror::FrontBack,
            ),
            (
                Rotation::Counterclockwise90,
                Mirror::None,
                "north",
                "west",
                mod_api::StructureRotation::Counterclockwise90,
                mod_api::StructureMirror::None,
            ),
            (
                Rotation::Counterclockwise90,
                Mirror::LeftRight,
                "north",
                "east",
                mod_api::StructureRotation::Counterclockwise90,
                mod_api::StructureMirror::LeftRight,
            ),
            (
                Rotation::Counterclockwise90,
                Mirror::FrontBack,
                "north",
                "west",
                mod_api::StructureRotation::Counterclockwise90,
                mod_api::StructureMirror::FrontBack,
            ),
        ] {
            let (world, _) = place(&resources, "test:machine", input, rotation, mirror);
            assert_eq!(
                calls.lock().unwrap().pop(),
                Some(mod_api::StructureTransform::new(api_rotation, api_mirror))
            );
            assert_eq!(
                world.state,
                BlockStateId(
                    NAMES
                        .iter()
                        .position(|n| *n == format!("test:{expected}"))
                        .unwrap() as u32
                )
            );
            assert_eq!(
                world.stored.unwrap().get(&"facing".into()),
                Some(&Tag::String(expected.into()))
            );
        }
        // Keep the east-facing mirror cases alongside the full north-facing matrix.
        for (mirror, expected) in [(Mirror::LeftRight, "east"), (Mirror::FrontBack, "west")] {
            let (world, _) = place(&resources, "test:machine", "east", Rotation::None, mirror);
            assert_eq!(
                world.stored.unwrap().get(&"facing".into()),
                Some(&Tag::String(expected.into()))
            );
            assert!(calls.lock().unwrap().pop().is_some());
        }
        for id in ["test:plain", "minecraft:chest"] {
            let (world, original) =
                place(&resources, id, "north", Rotation::Clockwise90, Mirror::None);
            let stored = world.stored.unwrap();
            assert_eq!(stored, original);
            assert_eq!(
                write_network(&Tag::Compound(stored), Limits::default()).unwrap(),
                write_network(&Tag::Compound(original), Limits::default()).unwrap()
            );
            assert!(calls.lock().unwrap().is_empty());
        }
        for (kind, visited) in [
            ("test:refuses", true),
            ("test:refuses", true),
            ("test:panics", true),
            ("test:panics", false),
        ] {
            let (world, mut expected) =
                place(&resources, kind, "north", Rotation::None, Mirror::None);
            if visited {
                expected.insert("visited".into(), Tag::Byte(1));
            }
            assert_eq!(world.stored, Some(expected));
        }
        // Poison is per callback, not per mod.
        let (world, _) = place(
            &resources,
            "test:machine",
            "north",
            Rotation::Clockwise90,
            Mirror::None,
        );
        assert_eq!(
            world.stored.unwrap().get(&"facing".into()),
            Some(&Tag::String("east".into()))
        );
        let logs = LOG.0.lock().unwrap();
        assert_eq!(
            logs.iter()
                .filter(
                    |line| line.contains("mod test: block entity test:refuses at [3, 64, 5]")
                        && line.contains("fixture refusal")
                )
                .count(),
            2
        );
        assert_eq!(
            logs.iter()
                .filter(
                    |line| line.contains("mod test panicked in hook block_entity_transform")
                        && line.contains("fixture panic")
                        && line.contains("disabled")
                )
                .count(),
            1
        );
        assert_eq!(
            mod_api::__private::take_hook_panics(),
            [mod_api::__private::HookPanicReport {
                mod_id: "test",
                hook: "block_entity_transform",
            }]
        );
    });
}

#[test]
fn structure_transform_unregistered_path_keeps_data_without_an_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let resources = resources(dir.path());
    assert!(
        resources
            .registry
            .structure_block_entity_transform
            .get()
            .is_none()
    );
    let (world, original) = place(
        &resources,
        "test:machine",
        "north",
        Rotation::Clockwise90,
        Mirror::FrontBack,
    );
    assert_eq!(world.state, BlockStateId(3));
    assert_eq!(world.stored, Some(original));
}

#[test]
fn structure_transform_hostile_output_keeps_original_identity() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(Calls::default())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resources = resources(dir.path());
        prepare(resources.registry);
        let (world, _) = place(
            &resources,
            "test:hostile",
            "north",
            Rotation::Clockwise90,
            Mirror::None,
        );
        let stored = world.stored.unwrap();
        assert_eq!(
            stored,
            [
                ("id".into(), Tag::String("test:hostile".into())),
                ("x".into(), Tag::Int(3)),
                ("y".into(), Tag::Int(64)),
                ("z".into(), Tag::Int(5)),
                ("facing".into(), Tag::String("east".into())),
            ]
            .into()
        );
    });
}

#[test]
fn structure_transform_reuses_the_unedited_template() {
    let calls = Calls::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resources = resources(dir.path());
        prepare(resources.registry);
        let input = [
            ("id".into(), Tag::String("test:machine".into())),
            ("facing".into(), Tag::String("north".into())),
            ("persistent".into(), Tag::Int(42)),
        ]
        .into();
        let template = template(&resources, "test:north", &input);
        for (rotation, state, facing) in [
            (Rotation::Clockwise90, BlockStateId(3), "east"),
            (Rotation::Clockwise180, BlockStateId(4), "south"),
        ] {
            let world = place_template(&resources, &template, rotation, Mirror::None);
            assert_eq!(world.state, state);
            assert_eq!(
                world.stored,
                Some(
                    [
                        ("id".into(), Tag::String("test:machine".into())),
                        ("facing".into(), Tag::String(facing.into())),
                        ("persistent".into(), Tag::Int(42)),
                    ]
                    .into()
                )
            );
        }
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                mod_api::StructureTransform::new(
                    mod_api::StructureRotation::Clockwise90,
                    mod_api::StructureMirror::None
                ),
                mod_api::StructureTransform::new(
                    mod_api::StructureRotation::Clockwise180,
                    mod_api::StructureMirror::None
                ),
            ]
        );
    });
}

#[test]
fn structure_transform_leaves_nbt_limits_to_serialization() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(Calls::default())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let resources = resources(dir.path());
        prepare(resources.registry);
        let (world, _) = place(
            &resources,
            "test:deep",
            "north",
            Rotation::None,
            Mirror::None,
        );
        let tag = Tag::Compound(world.stored.unwrap());
        let mut nested = tag.get("nested").unwrap();
        for _ in 0..514 {
            nested = nested.get("child").unwrap();
        }
        assert_eq!(nested, &Tag::Byte(1));
        // Placement accepts the live data. The depth rules belong to the actual packet
        // and disk writers.
        assert!(
            <sunrise_protocol::buf::OptionalCompoundTag as sunrise_protocol::buf::Codec<_>>::validate(
                &tag
            )
            .is_err(),
            "chunk block-entity codec refuses over-depth NBT"
        );
        assert!(
            sunrise_protocol::packets::play::Clientbound::BlockEntityData(
                sunrise_protocol::packets::play::BlockEntityData {
                    pos: [3, 64, 5],
                    kind: 0,
                    tag: tag.clone(),
                },
            )
            .try_encode()
            .is_err(),
            "block-entity update packet refuses over-depth NBT"
        );
        assert!(write_network(&tag, Limits::network()).is_err());
        assert!(
            ChunkRecord::from_nbt(
                &NamedTag {
                    name: "".into(),
                    tag
                },
                Compression::None,
                0,
                Limits::default(),
            )
            .is_ok()
        );
    });
}

#[test]
#[ignore = "daily"]
fn structure_transform_survives_real_datapack_reload_with_its_registry() {
    use crate::game::{native_entity_refs::tests::with_game, world_loading::WorldRequest};
    let calls = Calls::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Registration(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let dir = tempfile::tempdir().unwrap();
            let mut request = WorldRequest::new(
                "transform_reload",
                Some("transform_reload"),
                Some(1),
                true,
                None,
                Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = dir.path().to_owned();
            let prepared = game.prepare_world_and_rules(request).unwrap();
            game.finish_open_world(prepared).unwrap();
            let registry = game.pack.assets.registry;
            let frozen = registry
                .structure_block_entity_transform
                .get()
                .unwrap()
                .clone();
            let transforms = StateTransforms::from_json(
                &std::fs::read_to_string(
                    game.options
                        .resources
                        .join("worldgen/blocks/structure-blocks.json"),
                )
                .unwrap(),
                registry.len(),
            )
            .unwrap();
            let resources = StructureResources::new(
                registry,
                transforms,
                dir.path(),
                Default::default(),
                &Default::default(),
            );
            let check_placement = || {
                let (world, _) = place_block(
                    &resources,
                    "test:machine",
                    "north",
                    "minecraft:furnace",
                    Rotation::Clockwise90,
                    Mirror::None,
                );
                assert_eq!(
                    world.stored.unwrap().get(&"facing".into()),
                    Some(&Tag::String("east".into()))
                );
                assert_eq!(
                    registry.state(world.state).unwrap().properties["facing"],
                    "east"
                );
            };
            check_placement();
            assert!(game.run_command("reload"));
            assert!(std::ptr::eq(registry, game.pack.assets.registry));
            assert!(Arc::ptr_eq(
                &frozen,
                game.pack
                    .assets
                    .registry
                    .structure_block_entity_transform
                    .get()
                    .unwrap()
            ));
            check_placement();
            assert_eq!(
                *calls.lock().unwrap(),
                vec![
                    mod_api::StructureTransform::new(
                        mod_api::StructureRotation::Clockwise90,
                        mod_api::StructureMirror::None
                    );
                    2
                ]
            );
        })
    });
}
