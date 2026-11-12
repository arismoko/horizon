//! `light_emission` asked of a real part on a content block, through the game's own asks: only a
//! block declaring `dynamic_light` is asked, its answer reads the position, `None` keeps the
//! state's light, an answer above 15 is stored as 15, and a bulk replacement and a reload of
//! the chunk keep or ask again as the cells' states say.
use super::Game;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use horizon_client::chunk::Chunk;
use mod_api::*;
use std::sync::{Arc, Mutex};

const LAMP: &str = "lights:lamp";
const PLAIN: &str = "lights:plain";

/// Answers by height, as a crystal brighter deeper down would, and records where it is asked.
#[derive(Clone, Default)]
struct Depth(Arc<Mutex<Vec<[i32; 3]>>>);

#[mod_api::horizon_part]
impl BlockPart for Depth {
    fn light_emission(&self, _: &dyn Host, event: &mut LightEmissionEvent) -> Option<u8> {
        self.0
            .lock()
            .unwrap()
            .push([event.pos.x, event.pos.y, event.pos.z]);
        match event.pos.y {
            y if y < 0 => Some(9),
            64 => Some(12),
            65 => None,
            66 => Some(200),
            _ => Some(255),
        }
    }
}

/// A lamp declaring `dynamic_light`, and a block with the same light that doesn't; the same
/// part is attached to both.
struct Lights(Depth);

#[mod_api::__subscriptions]
impl Hooks for Lights {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for id in [LAMP, PLAIN] {
            let key = r.block_key(id);
            r.block_part(key, self.0.clone());
        }
    }

    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        for (id, dynamic) in [(LAMP, true), (PLAIN, false)] {
            let mut block = mod_api::Block::new(id, "minecraft:glass");
            block.properties.light = Some(4);
            block.properties.dynamic_light = dynamic.then_some(true);
            event.blocks.push(block);
        }
    }
}

/// The stored answer of the cell at (1, y, 2) of chunk (0, 0).
fn answer(game: &Game<'_>, y: i32) -> Option<u8> {
    let chunk = game.world.chunks.get(&(0, 0)).unwrap();
    let cell = chunk.dynamic_cell(1, y, 2).unwrap();
    chunk.dynamic_emission.get(&cell).copied()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_dynamic_light_part_answers_by_position_for_declared_blocks_only() {
    if run_fresh_cases(
        "game::dynamic_light_emission_tests::a_dynamic_light_part_answers_by_position_for_declared_blocks_only",
        "HORIZON_DYNAMIC_LIGHT_CHILD",
        &["game"],
    ) {
        return;
    }
    let depth = Depth::default();
    let mods: Mods = vec![ModEntry::new("lights", Box::new(Lights(depth.clone())))].into();
    mod_api::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        let registry = &game.pack.assets.registry;
        let (lamp, plain, air) = (
            registry.default_state(LAMP).unwrap(),
            registry.default_state(PLAIN).unwrap(),
            registry.default_state("minecraft:air").unwrap(),
        );
        let light = &game.pack.context.light;
        assert!(light.is_dynamic(lamp.0));
        assert!(!light.is_dynamic(plain.0), "the declaration gates the ask");
        assert_eq!(light.emission(lamp.0), 4);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.light_edits.clear();
        for (y, state) in [(-10, lamp), (64, lamp), (65, lamp), (66, lamp), (67, lamp)] {
            game.world
                .set_with_notification(1, y, 2, state, false)
                .unwrap();
        }
        game.world
            .set_with_notification(1, 68, 2, plain, false)
            .unwrap();
        game.ask_edited_light_emission();
        assert_eq!(
            std::mem::take(&mut *depth.0.lock().unwrap()),
            [[1, -10, 2], [1, 64, 2], [1, 65, 2], [1, 66, 2], [1, 67, 2]],
            "the undeclared block at y 68 is not asked"
        );
        // Deeper is brighter; None keeps the state's 4 (no entry); 200 and 255 are stored as 15.
        assert_eq!(
            [-10, 64, 65, 66, 67, 68].map(|y| answer(game, y)),
            [Some(9), Some(12), None, Some(15), Some(15), None]
        );

        // A bulk edit of the section holding y 64 to 67 replaces only the cell at y 66.
        let s = game
            .world
            .chunks
            .get(&(0, 0))
            .unwrap()
            .section_index(64)
            .unwrap();
        let chunk = Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap());
        let mut values: [u32; 4096] = std::array::from_fn(|i| chunk.sections[s].get(i));
        values[2 << 8 | 2 << 4 | 1] = air.0;
        chunk.replace_blocks(s, &values);
        assert_eq!(
            [-10, 64, 66, 67].map(|y| answer(game, y)),
            [Some(9), Some(12), None, Some(15)]
        );

        // The column comes in again, its answers gone: every lamp is asked once more.
        let mut reloaded = (**game.world.chunks.get(&(0, 0)).unwrap()).clone();
        reloaded.dynamic_emission.clear();
        reloaded.dynamic_pending = true;
        game.world.chunks.remove(&(0, 0));
        game.world.chunks.insert((0, 0), Arc::new(reloaded));
        game.ask_chunk_light_emission((0, 0));
        let mut asked = std::mem::take(&mut *depth.0.lock().unwrap());
        asked.sort();
        assert_eq!(asked, [[1, -10, 2], [1, 64, 2], [1, 65, 2], [1, 67, 2]]);
        assert_eq!(
            [-10, 64, 65, 66, 67].map(|y| answer(game, y)),
            [Some(9), Some(12), None, None, Some(15)]
        );
        assert!(!game.world.chunks.get(&(0, 0)).unwrap().dynamic_pending);
    });
}
