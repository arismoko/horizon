use std::sync::{Arc, Mutex};
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_sim::mobs::Mob;
use {
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

struct BenchMod(Arc<Mutex<Option<mod_api::AttachmentKey<u32>>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for BenchMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = r
            .register_attachment_type(mod_api::AttachmentDescriptor::new(
                "bench:tick_state",
                || 0u32,
            ))
            .unwrap();
        *self.0.lock().unwrap() = Some(key);
    }
}

#[test]
#[ignore = "callgrind fixture; needs SUNRISE_TEST_RESOURCES"]
fn attachment_instruction_count() {
    let Some(passes) = std::env::var("ATTACHMENT_PASSES")
        .ok()
        .and_then(|passes| passes.parse::<usize>().ok())
    else {
        eprintln!("refused, needs ATTACHMENT_PASSES; run by the parity harness");
        return;
    };
    let key_slot = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bench",
        Box::new(BenchMod(key_slot.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let key = key_slot
                .lock()
                .unwrap()
                .clone()
                .expect("BenchMod registered");
            game.options.dedicated = Some(Vec::new());
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.bind_dimension_attachments(game.dim);
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut holders = Vec::new();
            for id in 100..400 {
                game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
                holders.push(mod_api::Holder::Entity(id));
            }
            for z in (0..40).step_by(2) {
                for x in 0..20 {
                    game.world.block_entities.insert(
                        (x, 201, z),
                        sunrise_gameplay_common::game::BlockEntity::Chest(vec![None; 5]),
                    );
                    holders.push(mod_api::Holder::BlockEntity([x, 201, z]));
                }
            }
            for &holder in &holders {
                key.set(game, holder, 0)
                    .unwrap_or_else(|e| panic!("warm holder {holder:?}: {e:?}"));
            }
            assert_eq!(
                game.attachment_indexes[&game.dim]
                    .lock()
                    .unwrap()
                    .snapshot_named("bench", "tick_state")
                    .len(),
                700
            );
            attachment_measured_passes(game, &key, &holders, passes);
            for &holder in &holders {
                assert_eq!(key.get(game, holder, Clone::clone).unwrap(), passes as u32);
            }
        });
    });
}

#[inline(never)]
fn attachment_measured_passes(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    key: &mod_api::AttachmentKey<u32>,
    holders: &[mod_api::Holder],
    passes: usize,
) {
    for _ in 0..passes {
        for &holder in holders {
            let value = key.get(game, holder, Clone::clone).unwrap();
            key.set(game, holder, std::hint::black_box(value.wrapping_add(1)))
                .unwrap();
        }
    }
}
