use crate::game::ServerGame;
use sunrise_gameplay_common::game::{api_values, cauldron_interactions::Callback};
use sunrise_sim::first_person_sim::Hand;

impl ServerGame<'_> {
    /// Invoke a mod's authoritative cauldron interaction on the server.
    pub(in crate::game::server_side) fn use_mod_cauldron(
        &mut self,
        pos: [i32; 3],
        under_water: bool,
        callback: &Callback,
    ) -> sunrise_sim::cauldron::Outcome {
        use mod_api::{CauldronDecision, CauldronUse};
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        use sunrise_sim::cauldron::Outcome;
        let Some(guard) = callback.guard.as_ref() else {
            log::warn!(
                "cauldron interaction for mod {} has no installed owner",
                callback.owner
            );
            return Outcome::Consume;
        };
        let input = (|| -> std::result::Result<CauldronUse, String> {
            let held = self
                .common
                .held()
                .cloned()
                .ok_or_else(|| "missing held stack".to_owned())?;
            let before = self.common.world.block(pos[0], pos[1], pos[2]);
            let index = api_values::block_state::current()
                .map_err(|err| format!("block state index unavailable: {err:?}"))?;
            let state = index
                .to_api(before)
                .map_err(|err| format!("block state {before:?} unavailable: {err:?}"))?;
            let epoch = self
                .common
                .native_lifecycle
                .epoch()
                .ok_or_else(|| "no active world epoch".to_owned())?;
            let player_id = self.native_acting_id();
            let player = self.player(player_id).unwrap_or_else(|| {
                // A fake player can be the acting body without an online-player entry.
                let actor = mod_api::EntityRef::new(epoch, self.native_acting_entity());
                mod_api::__private::game::player_ref(actor, player_id)
            });
            let level = self.level_ref(epoch, self.common.dim.key());
            let held = held
                .try_into_api()
                .map_err(|err| format!("held stack projection failed: {err:?}"))?;
            Ok(CauldronUse {
                level,
                pos,
                state,
                player,
                hand: match self.common.hand {
                    Hand::Main => mod_api::Hand::Main,
                    Hand::Off => mod_api::Hand::Off,
                },
                held,
                underwater: under_water,
                creative: self.common.creative,
            })
        })();
        let input = match input {
            Ok(input) => input,
            Err(why) => {
                log::warn!(
                    "cauldron interaction input refused for mod {}: {why}",
                    callback.owner
                );
                return Outcome::Consume;
            }
        };
        let decision = guard.call(|| callback.handler.interact(self, &input));
        match decision {
            Ok(CauldronDecision::Success) => Outcome::Success,
            Ok(CauldronDecision::Consume) => Outcome::Consume,
            Ok(_) => Outcome::Pass,
            Err(why) => {
                log::warn!("cauldron interaction refused: {why:?}");
                Outcome::Consume
            }
        }
    }
}

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
