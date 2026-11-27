use crate::game::ServerGame;
use sunrise_sim::mobs::Who;

impl ServerGame<'_> {
    /// A named name tag names the mob under the crosshair.
    pub(in crate::game::server_side) fn name_mob(&mut self, index: usize) -> bool {
        // `NameTagItem`: the mob takes the tag's `custom_name` component, style and all.
        let Some((name, text)) = self
            .common
            .held()
            .filter(|h| &*h.item == "minecraft:name_tag")
            .and_then(|h| {
                let text = h.data()?.other("minecraft:custom_name");
                Some((
                    h.custom_name()?,
                    text.map(sunrise_sim::entity_nbt::name_text),
                ))
            })
        else {
            return false;
        };
        let m = &mut self.common.mobs[index];
        if !m.alive() {
            return false;
        }
        let entity_id = m.id;
        let entity_name = name.clone();
        m.custom_name = Some(name);
        m.custom_name_text = text;
        m.persistent = true;
        let at = [m.position.x, m.position.y, m.position.z];
        // itemStack.shrink(1); `Player.interactOn` puts a creative player's
        // count back, which `consume_held` skips for them.
        self.consume_held();
        // `Mob.interact`: the tag consumed the use, so
        // `gameEvent(GameEvent.ENTITY_INTERACT, player)` at the mob.
        let ctx = self.who_context(Some(Who::Player), None);
        self.post_game_event("minecraft:entity_interact", at, ctx);
        self.swing_arm();
        if let (Some(player), Some(player_name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            self.native_player_name_entity(player, &player_name, entity_id, &entity_name);
        }
        true
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
