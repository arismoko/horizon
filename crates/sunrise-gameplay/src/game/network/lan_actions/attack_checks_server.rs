use sunrise_gameplay_common::game::lan_actions::attack_checks::arrow_attackable;
use sunrise_protocol::text::translatable;

impl crate::game::ServerGame<'_> {
    /// Whether a guest's attack on entity `id` ends before `Player.attack` (handleAttack).
    /// The guest who attacks an `ItemEntity`, an `ExperienceOrb`, themselves or an
    /// `AbstractArrow` that is not `isAttackable` is disconnected with
    /// `multiplayer.disconnect.invalid_entity_attacked`.
    pub(in crate::game::server_side) fn guest_attack_stops(&mut self, gi: usize, id: i32) -> bool {
        use crate::game::commands::Target;
        let Some(host) = self.server.lan.as_ref() else {
            return true;
        };
        if !self.guest_in_place(gi) {
            return true;
        }
        let guest = &host.hub.guests[gi];
        let conn = guest.conn;
        let target = if guest.entity_id == id {
            Some(guest.bounds())
        } else {
            self.level_entity(id).and_then(|target| {
                let refused = match target {
                    Target::Item(_) | Target::Orb(_) => true,
                    Target::Projectile(_) | Target::Trident(_) => {
                        !self.arrow_attackable(&self.target_type(target))?
                    }
                    _ => false,
                };
                refused
                    .then(|| self.selector_target_bounds(target))
                    .flatten()
            })
        };
        let Some(target) = target else {
            return false;
        };
        if !guest.client_loaded()
            || !self.inside_border(&target)
            || self.as_guest_at(gi, conn, |g| {
                g.within_attack_range(&target) && !g.holds_piercing_weapon()
            }) != Ok(true)
        {
            return true;
        }
        let host = self.server.lan.as_mut().unwrap();
        let name = host.hub.guests[gi].name.clone();
        host.hub.kick(
            conn,
            translatable("multiplayer.disconnect.invalid_entity_attacked", &[]),
        );
        log::warn!("Player {name} tried to attack an invalid entity");
        true
    }

    /// Whether guest `gi` is in the level in place and has not left: a connected guest here,
    /// or a fake placed in it.
    pub(in crate::game) fn guest_in_place(&self, gi: usize) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let guest = &host.hub.guests[gi];
        let present = host.hub.here(guest)
            || (guest.is_fake() && guest.placed && guest.dimension == host.hub.config.dimension);
        present && guest.left_reason.is_empty()
    }

    /// `Player.cannotAttack`'s `Entity.isAttackable` for entity `id`: false for an
    /// `ItemEntity`, an `ExperienceOrb`, a `FallingBlockEntity`, an `EyeOfEnder`, a
    /// `FireworkRocketEntity` and an `AbstractArrow` that is not attackable, where
    /// `Player.attack` ends before anything.
    pub(super) fn cannot_attack(&self, id: i32) -> bool {
        if self
            .common
            .falling
            .iter()
            .any(|block| block.id as i32 == id)
        {
            return true;
        }
        let Some(target) = self.level_entity(id) else {
            return false;
        };
        let kind = self.target_type(target);
        match kind.as_str() {
            "minecraft:item"
            | "minecraft:experience_orb"
            | "minecraft:eye_of_ender"
            | "minecraft:firework_rocket" => true,
            _ => self.arrow_attackable(&kind) == Some(false),
        }
    }

    /// `AbstractArrow.isAttackable` for an entity of type `kind`, None for a type that is no
    /// arrow (an arrow, a spectral arrow or a trident).
    fn arrow_attackable(&self, kind: &str) -> Option<bool> {
        if !matches!(
            kind,
            "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident"
        ) {
            return None;
        }
        let sync = self.server.lan.as_ref()?.hub.lan.shared.sync();
        Some(arrow_attackable(
            &sync.tags,
            sync.id("minecraft:entity_type", kind),
        ))
    }
    /// The main-hand stack has `minecraft:piercing_weapon` (a spear's).
    pub(in crate::game::server_side) fn holds_piercing_weapon(&self) -> bool {
        self.common.held().is_some_and(|stack| {
            self.common
                .pack
                .item_models
                .components_view(&sunrise_sim::item_models::stack_view(stack))
                .contains_key("minecraft:piercing_weapon")
        })
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
