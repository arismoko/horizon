//! Client-role bucket target prediction tests for mod `BlockPart::can_place_liquid`.
use std::sync::Arc;
use sunrise_level::chunk::Chunk;
use sunrise_world::{fluid::FluidKind, level::BucketContent};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::Hit,
};

const AT: [i32; 3] = [2, 65, 2];

fn key(id: &str) -> mod_api::FluidKey {
    mod_api::__private::fluid_key(id).unwrap()
}

#[derive(Default)]
struct LiquidCalls {
    queried: std::sync::Mutex<Vec<(bool, Option<u32>, mod_api::FluidKey)>>,
    placed: std::sync::Mutex<Vec<mod_api::FluidState>>,
}

struct LiquidPart(Arc<LiquidCalls>);

#[mod_api::sunrise_part]
impl mod_api::BlockPart for LiquidPart {
    fn can_place_liquid(
        &self,
        level: mod_api::LevelSide<'_>,
        event: &mut mod_api::CanPlaceLiquidEvent,
    ) -> Option<bool> {
        self.0.queried.lock().unwrap().push((
            matches!(level, mod_api::LevelSide::Client(_)),
            event.user.map(mod_api::LivingRef::id),
            event.fluid,
        ));
        Some(true)
    }

    fn place_liquid(
        &self,
        _level: &mut dyn mod_api::LevelWriteHost,
        event: &mut mod_api::PlaceLiquidEvent,
    ) -> Option<bool> {
        self.0.placed.lock().unwrap().push(event.fluid);
        Some(false)
    }
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for LiquidPart {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("minecraft:dandelion");
        r.block_part(block, LiquidPart(self.0.clone()));
    }
}

#[test]
#[ignore = "daily"]
fn liquid_part_predicts_the_same_target_on_connected_client() {
    let calls = Arc::new(LiquidCalls::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "liquid_container_client",
        Box::new(LiquidPart(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "LiquidPrediction").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 91;
            remote.profile = 42;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let dandelion = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dandelion")
                .unwrap();
            game.world.set(AT[0], AT[1], AT[2], dandelion).unwrap();
            let hit = Hit {
                pos: AT,
                face: [0, 1, 0],
                point: [2.5, 65.5, 2.5],
                inside: false,
            };
            let lava = BucketContent::Fluid(FluidKind::Lava);
            assert_eq!(game.bucket_target(lava, &hit), AT);
            assert_eq!(
                *calls.queried.lock().unwrap(),
                [(true, Some(91), key("minecraft:lava"))]
            );
            assert!(calls.placed.lock().unwrap().is_empty());
        });
    });
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
