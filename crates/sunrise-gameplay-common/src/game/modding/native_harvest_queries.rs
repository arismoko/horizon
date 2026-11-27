#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use mod_api::{
        CanHarvestBlockEvent, Flow, Hooks, HostSide, PlayerBreakSpeedEvent,
        PlayerHarvestCheckEvent, PlayerRef,
    };
    use std::sync::{Arc, Mutex};

    /// Asks the player query for a different state from a `player_harvest_check` listener.
    pub struct Asker(pub Arc<Mutex<Vec<(PlayerRef, Option<bool>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Asker {
        fn player_harvest_check(&self, host: HostSide<'_>, _: &mut PlayerHarvestCheckEvent) {
            let HostSide::Server(host) = host else { return };
            let player = host.players().acting_player().unwrap();
            let level = player.level(&*host).unwrap();
            let dirt = host.block_state_id("minecraft:dirt").unwrap();
            let asked = player.has_correct_tool_for_drops(host, level, [0, 64, 0], dirt);
            self.0.lock().unwrap().push((player, asked));
        }
    }

    /// Asks the destroy-progress query from inside each hook the query runs, and records the
    /// answer: every one is None, so the hook cannot run itself again without bound.
    fn ask_recursively(
        host: &mut dyn mod_api::ServerHost,
        seen: &Mutex<Vec<(&'static str, Option<f32>)>>,
        hook: &'static str,
    ) {
        let Some(player) = host.players().acting_player() else {
            return;
        };
        let Some(level) = player.level(host) else {
            return;
        };
        let stone = host.block_state_id("minecraft:stone").unwrap();
        let asked = player.destroy_progress(host, level, [0, 64, 0], stone);
        seen.lock().unwrap().push((hook, asked));
    }

    /// The per-block half of `can_harvest_block`, on stone.
    struct RecursivePart(Arc<Mutex<Vec<(&'static str, Option<f32>)>>>);

    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for RecursivePart {
        fn can_harvest_block(&self, host: HostSide<'_>, _: &mut CanHarvestBlockEvent) {
            if let HostSide::Server(host) = host {
                ask_recursively(host, &self.0, "can_harvest_block");
            }
        }
    }

    /// The global half: the harvest check and the break speed, and the stone part above.
    #[derive(Default)]
    pub struct RecursiveAsker(pub Arc<Mutex<Vec<(&'static str, Option<f32>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for RecursiveAsker {
        fn player_harvest_check(&self, host: HostSide<'_>, _: &mut PlayerHarvestCheckEvent) {
            if let HostSide::Server(host) = host {
                ask_recursively(host, &self.0, "player_harvest_check");
            }
        }

        fn player_break_speed(&self, host: HostSide<'_>, _: &mut PlayerBreakSpeedEvent) -> Flow {
            if let HostSide::Server(host) = host {
                ask_recursively(host, &self.0, "player_break_speed");
            }
            Flow::Continue
        }

        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, RecursivePart(Arc::clone(&self.0)));
        }
    }
}
