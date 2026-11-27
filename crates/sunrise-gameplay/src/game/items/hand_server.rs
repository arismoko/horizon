use crate::game::ServerGame;
use {
    sunrise_items::items::ItemStack, sunrise_sim::first_person_sim::Hand,
    sunrise_sim::first_person_sim::SwingAnimation, sunrise_sim::first_person_sim::swing_duration,
};

impl ServerGame<'_> {
    /// The item whose attack damage and speed modifiers the player has (`attack_item`).
    pub(in crate::game::server_side) fn attack_item_info(
        &self,
    ) -> Option<&sunrise_items::items::ItemInfo> {
        self.items().get(self.server.attack_item.as_deref()?)
    }
    /// Server-side `LivingEntity.swing` handler called via `side_ops::presentation`.
    pub(in crate::game) fn server_swing_hand(
        &mut self,
        hand: Hand,
        animation: SwingAnimation,
    ) -> bool {
        if self.native_entity_swing_event(hand) {
            return false;
        }
        let effects = &self.common.potions.effects;
        let dig_speed = effects
            .amplifier("haste")
            .max(effects.amplifier("conduit_power"));
        let fatigue = effects.amplifier("mining_fatigue");
        let duration = swing_duration(animation, dig_speed, fatigue);
        if self.common.swing.start_if_able(hand, animation, duration) {
            self.common.swings_started += 1;
        }
        true
    }
    /// `swing_arm` for a use whose result is `SwingSource.PREDICTED`: the
    /// player's client swings by itself, so a server keeps it to the others.
    pub(in crate::game::server_side) fn swing_arm_predicted(&mut self) {
        let before = self.common.swings_started;
        self.swing_arm();
        if self.common.swings_started != before {
            self.server.predicted_swings += 1;
        }
    }
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn tick_hand_server(
        &mut self,
        reset: bool,
        changed: bool,
        main: Option<&ItemStack>,
    ) {
        if reset {
            self.server.attack_ticks = 0;
        }
        if changed {
            self.apply_equipment();
        }
        self.server.attack_item = main.map(|s| s.item.clone());
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::item_attack::ItemAttackGameExt2 as _;
