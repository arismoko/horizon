//! `can_take_item_through_face`: a mod container's part decides what a hopper below pulls.
use super::*;
use crate::game::{ServerGame, native_entity_refs::tests::with_game};
use mod_api::{
    BlockEntityPart, BlockPart, CanTakeItemThroughFaceEvent, Hooks, Host, ModEntry, Mods,
    Registrar, RegistrationContext,
};
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
use sunrise_level::{chunk::Chunk, containers::ModBlockEntity};

const TARGET: Pos = (8, 64, 8);
const BELOW: Pos = (8, 63, 8);

/// Records the face each query came through.
#[derive(Clone, Default)]
struct Gate(Arc<std::sync::Mutex<Vec<mod_api::Direction>>>);

#[mod_api::sunrise_part]
impl BlockPart for Gate {}

#[mod_api::sunrise_part]
impl BlockEntityPart for Gate {
    fn can_take_item_through_face(
        &self,
        _: &dyn Host,
        event: &mut CanTakeItemThroughFaceEvent<'_>,
    ) -> Option<bool> {
        self.0.lock().unwrap().push(event.face);
        match event.slot {
            0 => Some(false),
            1 => None,
            _ => Some(true),
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for Gate {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let gated = r.declare_block_entity_type("gated", Vec::new());
        r.block_entity_part(&gated, self.clone()).unwrap();
    }
}

/// Slot 0 refused by the part, slot 1 kept as declared (an output), slot 2 allowed by the
/// part though not an output: the hopper below takes slot 1, then slot 2, never slot 0.
#[test]
#[ignore = "daily"]
fn a_hopper_below_pulls_only_what_the_part_lets_through() {
    let gate = Gate::default();
    let mods: Mods = vec![ModEntry::new("take", Box::new(gate.clone()))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let barrel = registry.default_state("minecraft:barrel").unwrap();
            game.world
                .set(TARGET.0, TARGET.1, TARGET.2, barrel)
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    barrel.0,
                    Arc::new(ModContainer {
                        slots: 3,
                        faces: std::array::from_fn(|_| vec![0, 1, 2]),
                        outputs: vec![0, 1],
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            let mut entity = ModBlockEntity::new("take:gated".into(), Default::default());
            entity.items = vec![
                Some(ItemStack::new("minecraft:stone", 1)),
                Some(ItemStack::new("minecraft:dirt", 1)),
                Some(ItemStack::new("minecraft:sand", 1)),
            ];
            game.world
                .block_entities
                .insert(TARGET, BlockEntity::Mod(Box::new(entity)));
            let hopper = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:hopper".to_owned(),
                    properties: [("facing".to_owned(), "down".to_owned())]
                        .into_iter()
                        .collect(),
                })
                .unwrap();
            game.world.set(BELOW.0, BELOW.1, BELOW.2, hopper).unwrap();
            game.world
                .block_entities
                .insert(BELOW, BlockEntity::Chest(vec![None; HOPPER_SLOTS]));

            let held = |game: &ServerGame<'_>| {
                let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&TARGET) else {
                    panic!("expected the mod block entity");
                };
                entity
                    .items
                    .iter()
                    .map(|s| s.as_ref().map(|s| s.item.to_string()))
                    .collect::<Vec<_>>()
            };
            game.tick_hopper(BELOW);
            assert_eq!(
                held(game),
                [
                    Some("minecraft:stone".to_owned()),
                    None,
                    Some("minecraft:sand".to_owned())
                ]
            );
            game.server.hoppers.get_mut(&BELOW).unwrap().cooldown = 0;
            game.tick_hopper(BELOW);
            assert_eq!(held(game), [Some("minecraft:stone".to_owned()), None, None]);
            game.server.hoppers.get_mut(&BELOW).unwrap().cooldown = 0;
            game.tick_hopper(BELOW);
            assert_eq!(held(game), [Some("minecraft:stone".to_owned()), None, None]);
            let faces = gate.0.lock().unwrap();
            assert!(!faces.is_empty());
            assert!(faces.iter().all(|face| *face == mod_api::Direction::Down));
        });
    });
}
