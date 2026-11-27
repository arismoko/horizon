use mod_api::{BlockPart, FenceGateInWallEvent, Hooks, LevelSide, Registrar};
use std::sync::Arc;
use sunrise_core::position::BlockPos;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;

struct Wall(bool);
#[mod_api::sunrise_part]
impl BlockPart for Wall {
    fn fence_gate_in_wall(&self, _: LevelSide<'_>, _: &mut FenceGateInWallEvent) -> Option<bool> {
        Some(self.0)
    }
}
struct Neighbours;
#[mod_api::__subscriptions]
impl Hooks for Neighbours {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (name, answer) in [
            ("minecraft:stone", true),
            ("minecraft:cobblestone_wall", false),
        ] {
            let key = r.block_key(name);
            r.block_part(key, Wall(answer));
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_gate_asks_its_neighbours_parts_for_wall_height() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("walls", Box::new(Neighbours))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let gate = registry.default_state("minecraft:oak_fence_gate").unwrap();
            for (z, name, expected) in [
                (0, "minecraft:stone", "true"),
                (2, "minecraft:cobblestone_wall", "false"),
                (4, "minecraft:mossy_cobblestone_wall", "true"),
                (6, "minecraft:glass", "false"),
            ] {
                let neighbour = registry.default_state(name).unwrap();
                game.with_level(|b, level, ticks| {
                    use sunrise_core::geometry::{Direction, Vec3};
                    use sunrise_world::level::{PlaceContext, PlacementSource};
                    let pos = BlockPos::new(0, 64, z);
                    let context = PlaceContext {
                        clicked_pos: pos,
                        face: Direction::Up,
                        hit: Vec3::new(0.5, 64.0, f64::from(z) + 0.5),
                        inside: false,
                        source: PlacementSource::Player {
                            yaw: 0.0,
                            pitch: 0.0,
                            sneaking: false,
                        },
                        item: "minecraft:oak_fence_gate".into(),
                        can_use_game_master_blocks: false,
                    };
                    b.set_block(level, ticks, BlockPos::new(1, 64, z), neighbour, 3);
                    for authoritative in [true, false] {
                        level.native_authoritative = authoritative;
                        let placed = b
                            .state_for_placement(level, ticks, b.table.block(gate), &context)
                            .unwrap()
                            .unwrap();
                        assert_eq!(
                            registry.state(placed).unwrap().properties["in_wall"],
                            expected
                        );
                    }
                    level.native_authoritative = true;
                    b.set_block(level, ticks, pos, gate, 3);
                    // Replacing the neighbour runs updateShape after placement.
                    b.set_block(
                        level,
                        ticks,
                        BlockPos::new(1, 64, z),
                        registry.default_state("minecraft:air").unwrap(),
                        3,
                    );
                    b.set_block(level, ticks, BlockPos::new(1, 64, z), neighbour, 3);
                });
                assert_eq!(
                    registry
                        .state(game.world.block(0, 64, z))
                        .unwrap()
                        .properties["in_wall"],
                    expected
                );
            }
        });
    });
}
