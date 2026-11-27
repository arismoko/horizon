/// The remote player animation uses the scoped item answer once per tick.
#[test]
#[ignore = "daily"]
fn remote_player_use_animation_dispatches_once() {
    use mod_api::{Hooks, Host, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Clone)]
    struct Part(Arc<AtomicUsize>);
    #[mod_api::sunrise_part]
    impl ItemPart for Part {
        fn use_animation(&self, _: &dyn Host, event: &mut UseAnimationEvent<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
            *event.animation = mod_api::UseAnimation::Drink;
        }
    }
    struct Mod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let item = r.item_key("minecraft:bow");
            r.item_part(item, Part(self.0.clone()));
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "remote_animation",
        Box::new(Mod(calls.clone())),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "animation-client").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.entity_id = 42;
            remote.level_players.insert(66, 789);
            let kind = remote.player_type.as_ref().unwrap().clone();
            let mut mob =
                sunrise_sim::mobs::Mob::new(66, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.using_item = 1;
            mob.extra.set_slot_stack(
                sunrise_items::equipment::MAINHAND,
                Some(sunrise_items::items::ItemStack::new("minecraft:bow", 1)),
            );
            game.mobs.push(mob);
            game.tick_remote_entities(false);
            assert_eq!(
                game.mobs[0].extra.use_animation,
                mod_api::UseAnimation::Drink
            );
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        });
    });
}
