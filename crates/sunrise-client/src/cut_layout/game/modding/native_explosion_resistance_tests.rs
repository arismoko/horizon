//! Explosion resistance reads through native mod handles on a fresh game.
#![cfg(test)]
use mod_api::{
    Block, BlockPart, BlockState, BlocksEvent, EntityRef, ExplosionInteraction,
    ExplosionResistance::{self, NoResistance, Resists, Unavailable},
    ExplosionResistanceEvent, HostBlockStates, LevelHost, ModEntry, Registrar, RegistrationContext,
    ResistanceCells, ResistanceCellsError,
    wire::{Reader, Writer},
};
use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_level::chunk::Chunk;
use sunrise_sim::projectiles::ProjectileKind;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
};

/// The one position where [`Spot`] answers.
const SPOT: [i32; 3] = [6, 80, 1];

/// Dirt's part: 5000 at [`SPOT`] only, counting the asks.
#[derive(Clone, Default)]
struct Spot(Arc<AtomicI32>);
#[mod_api::sunrise_part]
impl BlockPart for Spot {
    fn explosion_resistance(
        &self,
        _: &dyn LevelHost,
        event: &mut ExplosionResistanceEvent<'_>,
    ) -> Option<f32> {
        self.0.fetch_add(1, Ordering::Relaxed);
        ([event.pos.x, event.pos.y, event.pos.z] == SPOT).then_some(5000.0)
    }
}
/// Oak planks' part: 7 everywhere, declared pure, counting the asks.
#[derive(Clone, Default)]
struct Pure(Arc<AtomicI32>);
#[mod_api::sunrise_part]
impl BlockPart for Pure {
    fn explosion_resistance(
        &self,
        _: &dyn LevelHost,
        _: &mut ExplosionResistanceEvent<'_>,
    ) -> Option<f32> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Some(7.0)
    }
    fn explosion_resistance_is_pure(&self) -> bool {
        true
    }
}
#[derive(Clone, Default)]
struct Resisting {
    pub spot: Spot,
    pure: Pure,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Resisting {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let dirt = r.block_key("minecraft:dirt");
        r.block_part(dirt, self.spot.clone());
        let planks = r.block_key("minecraft:oak_planks");
        r.block_part(planks, self.pure.clone());
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        let mut block = Block::new("test:blastproof", "minecraft:stone");
        block.properties.explosion_resistance = Some(42.0);
        event.register(block).unwrap();
    }
}

/// A server's overworld chunk (0, 0): from x 1 along y 80, z 1 stone, obsidian, bedrock,
/// water, air, dirt at [`SPOT`], dirt, the content block, then four oak planks on z 2.
fn row(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for (x, block) in [
        (1, "stone"),
        (2, "obsidian"),
        (3, "bedrock"),
        (4, "water"),
        (6, "dirt"),
        (7, "dirt"),
    ] {
        assert!(game.run_command(&format!("setblock {x} 80 1 minecraft:{block}")));
    }
    let content = game
        .pack
        .assets
        .registry
        .default_state("test:blastproof")
        .unwrap();
    game.world.set(8, 80, 1, content).unwrap();
    assert!(game.run_command("fill 1 80 2 4 80 2 minecraft:oak_planks"));
}

/// Reads a block blast of radius 4 at the row.
fn read(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: [i32; 3],
    source: Option<EntityRef>,
) -> Option<ExplosionResistance> {
    let level = game.level("minecraft:overworld").unwrap();
    let (center, radius, fire, interaction) = blast();
    level.explosion_resistance(game, pos, source, center, radius, fire, interaction)
}

/// `read` for every cell of `cells`.
fn reads(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    cells: &ResistanceCells,
    source: Option<EntityRef>,
) -> Result<Vec<ExplosionResistance>, ResistanceCellsError> {
    let level = game.level("minecraft:overworld").unwrap();
    let (center, radius, fire, interaction) = blast();
    level.explosion_resistances(game, cells, source, center, radius, fire, interaction)
}

fn blast() -> ([f64; 3], f32, bool, ExplosionInteraction) {
    ([4.5, 80.5, 1.5], 4.0, false, ExplosionInteraction::Block)
}

/// Runs `test` on a fresh game with the resisting mod.
fn with_resisting_mod(
    test: impl Fn(&mut sunrise_gameplay::game::ServerGame<'_>, &Resisting) + Send + Sync + 'static,
) {
    run_with_game_stack(move || {
        let parts = Resisting::default();
        let mods: mod_api::Mods = vec![ModEntry::new("test", Box::new(parts.clone()))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            // The content block needs a content root built with the mod's blocks.
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            // One build at a time: the tests share the root's folder.
            static BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let root = {
                let _build = BUILD
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
                    .unwrap()
            };
            with_game_role(
                root.prepared().to_path_buf(),
                ExecutionRole::IntegratedServer,
                |game| {
                    row(game);
                    test(game, &parts);
                },
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_block_state_reads_the_resistance_its_block_declares() {
    with_resisting_mod(|game, _| {
        let states: Vec<Option<BlockState>> = [
            "minecraft:stone",
            "minecraft:obsidian",
            "minecraft:bedrock",
            "minecraft:water",
            "minecraft:dirt",
            "test:blastproof",
        ]
        .into_iter()
        .map(|name| mod_api::prelude::BlockStatesHost::block_state_id(&*game, name))
        .collect();
        let got: Vec<Option<f32>> = states
            .into_iter()
            .map(|state| game.block_states().explosion_resistance(state.unwrap()))
            .collect();
        // Dirt's part answers 5000 at one position; the state's own value stays 0.5.
        assert_eq!(
            got,
            [
                Some(6.0),
                Some(1200.0),
                Some(3_600_000.0),
                Some(100.0),
                Some(0.5),
                Some(42.0)
            ]
        );
        // Stone has no state past its one: an ordinal no state has.
        let stone = mod_api::prelude::BlockStatesHost::block_state_id(&*game, "minecraft:stone");
        let mut bytes = Writer::new();
        bytes.put(&stone.unwrap().block());
        bytes.put(&u16::MAX);
        let unknown: BlockState = Reader::new(bytes.bytes()).take().unwrap();
        assert_eq!(game.block_states().explosion_resistance(unknown), None);
    });
}

#[test]
#[ignore = "daily"]
fn a_cell_reads_what_a_blast_meets_there() {
    with_resisting_mod(|game, parts| {
        let at = |x: i32| [x, 80, 1];
        let got: Vec<_> = (1..=8).map(|x| read(game, at(x), None).unwrap()).collect();
        assert_eq!(
            got,
            [
                Resists(6.0),
                Resists(1200.0),
                Resists(3_600_000.0),
                Resists(100.0),
                NoResistance,
                Resists(5000.0),
                Resists(0.5),
                Resists(42.0),
            ]
        );
        assert_eq!(
            parts.spot.0.load(Ordering::Relaxed),
            2,
            "each dirt asked once"
        );
        // A chunk that isn't loaded is never loaded for a read.
        assert_eq!(read(game, [100, 80, 100], None), Some(Unavailable));
        assert!(!game.world.chunks.contains_key(&(6, 6)));
        // A state the game doesn't know.
        game.world
            .set(9, 80, 1, sunrise_world::registry::BlockStateId(u32::MAX))
            .unwrap();
        assert_eq!(read(game, at(9), None), Some(Unavailable));
        // Above the level: void air.
        assert_eq!(read(game, [1, 400, 1], None), Some(NoResistance));
    });
}

#[test]
#[ignore = "daily"]
fn a_batched_read_answers_each_cell_in_order() {
    with_resisting_mod(|game, parts| {
        let cells = ResistanceCells::Box {
            min: [1, 80, 1],
            max: [8, 80, 2],
        };
        let got = reads(game, &cells, None).unwrap();
        let mut expected = vec![
            Resists(6.0),
            Resists(1200.0),
            Resists(3_600_000.0),
            Resists(100.0),
            NoResistance,
            Resists(5000.0),
            Resists(0.5),
            Resists(42.0),
        ];
        expected.extend([Resists(7.0); 4]);
        expected.extend([NoResistance; 4]);
        assert_eq!(got, expected);
        // The pure planks part is asked once for its four cells; dirt's once a cell.
        assert_eq!(parts.pure.0.load(Ordering::Relaxed), 1);
        assert_eq!(parts.spot.0.load(Ordering::Relaxed), 2);
        let columns = ResistanceCells::Columns {
            columns: vec![(3, 1), (100, 100), (3, 1)],
            min_y: 79,
            max_y: 80,
        };
        assert_eq!(
            reads(game, &columns, None).unwrap(),
            [
                NoResistance,
                Resists(3_600_000.0),
                Unavailable,
                Unavailable,
                NoResistance,
                Resists(3_600_000.0)
            ]
        );
        assert!(!game.world.chunks.contains_key(&(6, 6)));
        let past_old_cap = ResistanceCells::Box {
            min: [0, 0, 0],
            max: [63, 63, 64],
        };
        let large = reads(game, &past_old_cap, None).unwrap();
        assert_eq!(large.len(), 266_240);
        assert_eq!(large[0], NoResistance);
        assert_eq!(large[63], Unavailable);
        let overflow = ResistanceCells::Box {
            min: [i32::MIN; 3],
            max: [i32::MAX; 3],
        };
        assert_eq!(
            reads(game, &overflow, None),
            Err(ResistanceCellsError::Overflow)
        );
        assert_eq!(
            parts.pure.0.load(Ordering::Relaxed),
            1,
            "refused, nothing read"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_two_layer_box_reads_y_after_z_after_x() {
    with_resisting_mod(|game, _| {
        // [1, 81, 1] and [2, 81, 2] over the row.
        assert!(game.run_command("setblock 1 81 1 minecraft:obsidian"));
        assert!(game.run_command("setblock 2 81 2 minecraft:bedrock"));
        let cells = ResistanceCells::Box {
            min: [1, 80, 1],
            max: [2, 81, 2],
        };
        assert_eq!(
            reads(game, &cells, None).unwrap(),
            [
                // y 80: z 1 (stone, obsidian), z 2 (planks, planks)
                Resists(6.0),
                Resists(1200.0),
                Resists(7.0),
                Resists(7.0),
                // y 81: z 1 (obsidian, air), z 2 (air, bedrock)
                Resists(1200.0),
                NoResistance,
                NoResistance,
                Resists(3_600_000.0),
            ]
        );
    });
}

/// The source's calculator is the one `level.explode` uses: what a read says, the rays do.
/// `name` is the block at `at`, read with and without `source` and then blasted from beside it
/// with and without `source`; `kept` is whether it survives each blast.
fn read_and_blast(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    source: Option<EntityRef>,
    at: [i32; 3],
    expected: [ExplosionResistance; 2],
    kept: [bool; 2],
) {
    let level = game.level("minecraft:overworld").unwrap();
    let block = game.world.block(at[0], at[1], at[2]);
    let center = [
        f64::from(at[0]) + 0.5,
        f64::from(at[1]) + 0.5,
        f64::from(at[2]) + 1.5,
    ];
    for (i, source) in [source, None].into_iter().enumerate() {
        game.world.set(at[0], at[1], at[2], block).unwrap();
        assert_eq!(
            level.explosion_resistance(
                game,
                at,
                source,
                center,
                2.0,
                false,
                ExplosionInteraction::Block,
            ),
            Some(expected[i]),
            "{source:?}"
        );
        let settings = mod_api::ExplosionSettings::default();
        assert_eq!(
            level.explode(
                game,
                source,
                center,
                2.0,
                false,
                ExplosionInteraction::Block,
                settings
            ),
            Ok(())
        );
        let survived = game.world.block(at[0], at[1], at[2]) == block;
        assert_eq!(survived, kept[i], "{source:?}: the rays did as read");
    }
}

#[test]
#[ignore = "daily"]
fn a_reads_calculator_is_the_one_the_blasts_rays_use() {
    with_resisting_mod(|game, _| {
        // A dangerous wither skull caps obsidian at 0.8, and its blast breaks it; without it
        // obsidian resists 1200 and stays.
        assert!(game.run_command("summon minecraft:wither_skull 1.5 90 14.5 {dangerous:1b}"));
        let skull = game
            .creatures
            .projectiles
            .iter()
            .find(|p| matches!(p.kind, ProjectileKind::WitherSkull { dangerous: true }))
            .unwrap()
            .id;
        let skull = game.entity(skull);
        assert!(game.run_command("setblock 12 80 12 minecraft:obsidian"));
        read_and_blast(
            game,
            skull,
            [12, 80, 12],
            [Resists(0.8), Resists(1200.0)],
            [false, true],
        );
        // A primed TNT minecart's blast neither resists on nor breaks a rail; without it the
        // rail resists 0.7 and breaks.
        assert!(game.run_command("summon minecraft:tnt_minecart 2.5 80 12.5 {fuse:80s}"));
        let cart = game.carts.iter().next().unwrap().id;
        let cart = game.entity(cart);
        assert!(game.run_command("setblock 12 80 4 minecraft:stone"));
        assert!(game.run_command("setblock 12 81 4 minecraft:rail"));
        read_and_blast(
            game,
            cart,
            [12, 81, 4],
            [Resists(0.0), Resists(0.7)],
            [true, false],
        );
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
