/// A mod's own anvil menu runs the `anvil_update` listeners (#5610), as
/// `CommonHooks.onAnvilUpdate`: they see the menu's inputs and costs as vanilla's, for the
/// player it names, and their edits come back; a cancel is None, and a listener that asks
/// again from inside gets None instead of running them again.
#[test]
#[ignore = "daily"]
fn a_mods_anvil_menu_runs_the_anvil_update_listeners() {
    use mod_api::item::menu::{AnvilUpdateEdits, AnvilUpdateInputs};
    use mod_api::{
        AnvilUpdateEvent, Flow, Hooks, HostSide, ItemStack as ApiStack, ModEntry, StackResult,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Heard {
        /// The menu's player, which the listener asks again from inside.
        player: Option<mod_api::PlayerRef>,
        /// The player, the position, and what an inner `anvil_update` answered.
        calls: Vec<(u128, [i32; 3], Option<AnvilUpdateEdits>)>,
    }
    struct Recipes(Arc<Mutex<Heard>>);
    #[mod_api::__subscriptions]
    impl Hooks for Recipes {
        fn anvil_update(&self, host: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
            let HostSide::Server(host) = host else {
                return Flow::Continue;
            };
            let player = self.0.lock().unwrap().player.unwrap();
            let inner = player.anvil_update(host, event.pos, event.inputs);
            self.0
                .lock()
                .unwrap()
                .calls
                .push((event.player, event.pos, inner));
            if event.inputs.name == Some("cancel") {
                return Flow::Cancel;
            }
            event.edits.output = StackResult::Set(Some(ApiStack::of(event.inputs.left.item())));
            event.edits.xp_cost = event.inputs.vanilla_xp_cost * 2;
            event.edits.material_cost = 0;
            Flow::Continue
        }
    }

    let heard = Arc::new(Mutex::new(Heard::default()));
    let mods = vec![ModEntry::new("recipes", Box::new(Recipes(heard.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = crate::game::native_server::fake_action_tests::fake(game, None, 0);
            heard.lock().unwrap().player = Some(player);
            let left = ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
            let inputs = |name| AnvilUpdateInputs {
                left: &left,
                right: None,
                name: Some(name),
                vanilla_output: None,
                vanilla_xp_cost: 3,
                vanilla_material_cost: 1,
            };
            let edits = player.anvil_update(game, [7, 64, 7], &inputs("upgraded"));
            assert_eq!(
                heard.lock().unwrap().calls.len(),
                1,
                "the listener ran once"
            );
            assert_eq!(
                edits,
                Some(AnvilUpdateEdits {
                    output: StackResult::Set(Some(left.clone())),
                    xp_cost: 6,
                    material_cost: 0,
                })
            );
            assert_eq!(
                player.anvil_update(game, [7, 64, 7], &inputs("cancel")),
                None
            );
            assert_eq!(
                heard.lock().unwrap().calls,
                [(2, [7, 64, 7], None), (2, [7, 64, 7], None)],
                "the player and position reach the listener; an inner call declines"
            );
        });
    });
}
