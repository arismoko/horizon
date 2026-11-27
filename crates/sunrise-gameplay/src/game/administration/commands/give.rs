//! `/give`'s handing over of one stack.
use super::Target;
use mod_api::SoundSource;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::level_sounds_common::LevelSound;
use sunrise_items::items::ItemStack;

pub(in crate::game) enum GiveItem {
    Overflow(Option<[i32; 4]>),
    Fake,
}

impl crate::game::ServerGame<'_> {
    /// `GiveCommand.giveItem` adds the fake pickup before drawing the sound pitch.
    pub(in crate::game) fn give_pickup(&mut self, prototype: ItemStack) {
        let Some(eye) = self.common.eye_position() else {
            return;
        };
        let velocity = self.player_drop_motion(false);
        self.join_give_item(prototype, eye, velocity, GiveItem::Fake);
        let Some(player) = self.common.player.as_mut() else {
            return;
        };
        let at = [player.position.x, player.position.y, player.position.z];
        let r = &mut player.random;
        let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
        self.send_level_sound(
            LevelSound::new("minecraft:entity.item.pickup", at, 0.2, pitch)
                .in_source(SoundSource::Players),
            None,
        );
    }
    /// The receiving player's eyes, where what did not fit is thrown from.
    pub(in crate::game::server_side) fn give_eye(&self) -> [f64; 3] {
        let p = self
            .target_position(Target::Player)
            .unwrap_or(self.source_position());
        self.common.eye_position().unwrap_or([
            p[0],
            p[1] + self.server_target_eye_height(Target::Player),
            p[2],
        ])
    }
    pub(in crate::game) fn give_remainder(&mut self, rest: ItemStack, owner: Option<[i32; 4]>) {
        let eye = self.give_eye();
        let velocity = self.player_drop_motion(false);
        self.join_give_item(rest, eye, velocity, GiveItem::Overflow(owner));
    }
    /// What did not fit of a `/give` whose player died or left between taking the stack and
    /// the root commit that throws it: it lies where it would have been thrown from, at
    /// `eye` as it was when the stack was taken, still with no pickup delay and `owner` its
    /// only taker. The stack was already taken from its source, so it lands rather than
    /// being lost.
    pub(in crate::game) fn give_remainder_unheld(
        &mut self,
        rest: ItemStack,
        owner: Option<[i32; 4]>,
        eye: [f64; 3],
    ) {
        self.join_give_item(rest, eye, [0.0; 3], GiveItem::Overflow(owner));
    }
}
