//! Removal exposes the engine's default and still lets a block Part replace it.
use mod_api::{BlockPart, BlockState, Hooks, LevelHost, Registrar, RemoveBlockEvent};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

const BLOCK: &str = "removal_probe:block";

#[derive(Clone, Default)]
struct Probe {
    pub heard: Arc<Mutex<Vec<BlockState>>>,
    replace: Arc<AtomicBool>,
}
#[mod_api::sunrise_part]
impl BlockPart for Probe {
    fn remove_block(&self, _: &dyn LevelHost, event: &mut RemoveBlockEvent) -> Option<BlockState> {
        self.heard.lock().unwrap().push(event.replacement);
        // A registered state distinct from the dry default, without a second lookup.
        self.replace.load(Ordering::SeqCst).then_some(event.state)
    }
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let key = r.block_key(BLOCK);
        r.block_part(key, self.clone());
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(BLOCK, "minecraft:stone"))
            .unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn removal_exposes_default_and_honours_the_parts_answer() {
    if run_fresh_cases(
        "cut_layout::game::native_hooks::remove_block_tests::removal_exposes_default_and_honours_the_parts_answer",
        "SUNRISE_REMOVE_BLOCK_CHILD",
        &["game"],
    ) {
        return;
    }
    let probe = Probe::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "removal_probe",
        Box::new(probe.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let state = game.pack.assets.registry.default_state(BLOCK).unwrap();
            let air = game.res.facts().air;
            let pos = BlockPos::new(2, 64, 2);
            game.world.set(2, 64, 2, state).unwrap();
            let removed =
                game.with_level(|b, level, ticks| b.remove_block(level, ticks, pos, false));
            assert!(removed);
            assert_eq!(game.world.block(2, 64, 2), air);
            assert_eq!(*probe.heard.lock().unwrap(), [air.try_into_api().unwrap()]);

            // The Part receives the same dry default but overrides it with the old state.
            game.world.set(2, 64, 2, state).unwrap();
            probe.replace.store(true, Ordering::SeqCst);
            let removed =
                game.with_level(|b, level, ticks| b.remove_block(level, ticks, pos, true));
            assert!(
                !removed,
                "the single setBlock returns false for an equal state"
            );
            assert_eq!(game.world.block(2, 64, 2), state);
            assert_eq!(
                *probe.heard.lock().unwrap(),
                [air.try_into_api().unwrap(); 2]
            );
        });
    });
}
