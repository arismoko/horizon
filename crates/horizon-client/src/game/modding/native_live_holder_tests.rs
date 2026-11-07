//! Real-Game holder-boundary regressions and the ignored 1,000-ticker measurement.
use std::{sync::Arc, time::Instant};

use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
};
use mod_api::{
    BlockEntityPart, BlockEntityTickEvent, BlockPart, Hooks, HostTransaction, Registrar, ServerHost,
};

struct Ticker(bool);
#[mod_api::horizon_part]
impl BlockPart for Ticker {}
#[mod_api::horizon_part]
impl BlockEntityPart for Ticker {
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        if self.0 {
            let level = host.level("minecraft:overworld").unwrap();
            let sink = host.block_entity(level, [0, 63, 0]).unwrap();
            let value = sink.data_slots(host).unwrap()[0];
            let result: Result<(), ()> = host.transaction(|host, _| {
                assert!(sink.set_data_slot(host, 0, value + 1).unwrap());
                Ok(())
            });
            result.unwrap();
        }
        std::hint::black_box(event.pos);
    }
}
struct BenchMod(&'static str);
#[mod_api::__subscriptions]
impl Hooks for BenchMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("ticker").blocks(&[]).build();
        if self.0 != "none" {
            r.block_entity_part(&kind, Ticker(self.0 == "write"))
                .unwrap();
        }
    }
}

#[test]
#[ignore = "measurement; needs HORIZON_TEST_RESOURCES and HORIZON_LIVE_BENCH=none|trivial|write"]
fn thousand_live_block_entity_ticks() {
    let mode = std::env::var("HORIZON_LIVE_BENCH").unwrap();
    let mode = match mode.as_str() {
        "none" => "none",
        "trivial" => "trivial",
        "write" => "write",
        _ => panic!("unknown measurement mode"),
    };
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "live_bench",
        Box::new(BenchMod(mode)),
    )]
    .into();
    mod_api::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::mods::content_root(&base, &Default::default()).unwrap();
        super::native_entity_refs::tests::with_game_at(root.prepared().to_path_buf(), |game| {
            for x in 0..4 {
                for z in 0..4 {
                    game.world
                        .chunks
                        .insert((x, z), Arc::new(Chunk::empty(x, z)));
                }
            }
            let positions: Vec<_> = (0..1000).map(|i| (i % 40, 64, i / 40)).collect();
            for &pos in positions.iter().chain(std::iter::once(&(0, 63, 0))) {
                let mut entity =
                    ModBlockEntity::new("live_bench:ticker".into(), Default::default());
                entity.data = vec![0];
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Mod(Box::new(entity)));
            }
            let mut times = Vec::with_capacity(240);
            for round in 0..260 {
                let start = Instant::now();
                for &pos in &positions {
                    game.tick_mod_block_entity(pos);
                }
                let elapsed = start.elapsed().as_nanos();
                if round >= 20 {
                    times.push(elapsed);
                }
            }
            times.sort_unstable();
            if mode == "write" {
                let BlockEntity::Mod(sink) = game.world.block_entities.get(&(0, 63, 0)).unwrap()
                else {
                    panic!("sink removed")
                };
                assert_eq!(sink.data, [260_000]);
            }
            println!(
                "LIVE_BENCH mode={mode} holders=1000 samples={} median_ns={} p95_ns={} p99_ns={}",
                times.len(),
                times[120],
                times[228],
                times[237]
            );
        });
    });
}

pub(super) fn with_mod_parts(test: impl FnOnce(std::path::PathBuf)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "live_bench",
        Box::new(BenchMod("none")),
    )]
    .into();
    mod_api::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        test(
            super::mods::content_root(&base, &Default::default())
                .unwrap()
                .prepared()
                .to_owned(),
        );
    });
}

pub(super) fn mod_holder(game: &mut super::Game<'_>) -> mod_api::BlockEntityRef {
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(1, 64, 1, stone).unwrap();
    let holder = ModBlockEntity::new("live_bench:ticker".into(), Default::default());
    game.world
        .block_entities
        .insert((1, 64, 1), BlockEntity::Mod(Box::new(holder)));
    let level = game.level("minecraft:overworld").unwrap();
    game.block_entity(level, [1, 64, 1]).unwrap()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn data_slot_sizing_after_creation_and_load_joins_the_existing_journal() {
    use mod_api::BlockEntityDataError;
    with_mod_parts(|root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let pos = (1, 64, 1);
            assert_eq!(entity.data_slots(game), Some(vec![]));
            assert!(
                !entity.set_data_slot(game, 0, 1).unwrap(),
                "scalar writes cannot declare slots"
            );
            assert_eq!(entity.set_data_slots(game, vec![0]), Ok(true));
            let revision = |game: &super::Game<'_>| {
                let BlockEntity::Mod(entity) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("mod holder missing")
                };
                entity.revision()
            };
            let before_revision = revision(game);
            game.world.modified.clear();
            assert!(!entity.set_data_slot(game, 0, 0).unwrap());
            assert!(!entity.set_data_slot(game, 1, 1).unwrap());
            assert_eq!(entity.set_data_slots(game, vec![0]), Ok(false));
            assert_eq!(revision(game), before_revision);
            assert!(
                game.world.modified.is_empty(),
                "no-op sends no Changed notice"
            );
            assert_eq!(entity.data_slots(game), Some(vec![0]));
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, _| {
                assert_eq!(entity.set_data_slots(host, vec![30, 40]), Ok(true));
                host.transaction(|host, _| {
                    assert!(entity.set_data_slot(host, 0, 50).unwrap());
                    Ok(())
                })?;
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(entity.data_slots(game), Some(vec![0]));
            assert!(game.transactions.is_empty());
            assert_eq!(entity.set_data_slots(game, vec![0; 256]), Ok(true));
            assert!(entity.set_data_slot(game, 255, 7).unwrap());
            let before = entity.data_slots(game).unwrap();
            let before_revision = revision(game);
            game.world.modified.clear();
            assert_eq!(
                entity.set_data_slots(game, vec![0; 257]),
                Err(BlockEntityDataError::TooManySlots {
                    count: 257,
                    limit: 256
                })
            );
            assert_eq!(entity.data_slots(game), Some(before));
            assert_eq!(revision(game), before_revision);
            assert!(
                game.world.modified.is_empty(),
                "refusal sends no Changed notice"
            );
            let saved = horizon_client::save::encode_block_entity(
                pos,
                game.world.block_entities.get(&pos).unwrap(),
                "minecraft:stone",
            );
            let (loaded_pos, loaded) = horizon_client::save::decode_block_entity(&saved).unwrap();
            game.world.block_entities.remove(&pos);
            assert_eq!(
                entity.set_data_slots(game, vec![8]),
                Err(BlockEntityDataError::Stale)
            );
            assert_eq!(loaded_pos, pos);
            game.world.block_entities.insert(loaded_pos, loaded);
            assert_eq!(
                entity.data_slots(game),
                None,
                "old identity stays stale after load"
            );
            let entity = game
                .block_entity(game.level("minecraft:overworld").unwrap(), [1, 64, 1])
                .unwrap();
            assert_eq!(
                entity.data_slots(game),
                Some(vec![]),
                "menu data is transient, not saved NBT"
            );
            assert_eq!(entity.set_data_slots(game, vec![0]), Ok(true));
            assert!(entity.set_data_slot(game, 0, 1).unwrap());
            assert_eq!(entity.data_slots(game), Some(vec![1]));
        });
    });
}
