use super::*;

impl crate::game::ServerGame<'_> {
    /// `MinecartTNT.explode`: with explosions disabled, only a primed cart is
    /// discarded. An unprimed cart survives, including its current tick.
    pub fn explode_cart(&mut self, c: &mut Minecart, speed_sqr: f64) -> bool {
        let explodes = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES);
        if !explodes && c.fuse < 0 {
            return false;
        }
        let target = super::super::commands::Target::Cart(c.id);
        if self.common.persist.destroying_vehicles.contains(&target) {
            return true;
        }
        let power = if explodes {
            let speed = speed_sqr.sqrt().min(5.0);
            (f64::from(c.explosion_power)
                + f64::from(c.explosion_speed_factor) * c.random.next_double() * 1.5 * speed)
                as f32
        } else {
            0.0
        };
        let at = [c.position.x, c.position.y, c.position.z];
        // Publish the entire tick state, including the random draw, before
        // dismount hooks run. store_back transfers the resident owner's
        // attachment lifetime to the replacement; removing that owner later
        // also preserves any changes made by the hooks.
        if let Some(i) = self.common.carts.iter().position(|x| x.id == c.id)
            && !self.common.store_cart_back(i, c.clone())
        {
            return true;
        }
        // The cart is the explosion's source: it is gone before the blast
        // looks for entities, so it can't hit itself.
        let silent = explodes && self.entity_silent(c.id);
        if self
            .with_destroying_vehicle(target, |game| {
                if game.carts.iter().any(|x| x.id == c.id && x.join == c.join) {
                    game.release_vehicle_container(c.id);
                }
                if let Some(i) = game
                    .carts
                    .iter()
                    .position(|x| x.id == c.id && x.join == c.join)
                {
                    let removed = game.carts.remove(i);
                    game.retain_removed_vehicle(
                        c.id,
                        sunrise_gameplay_common::game::entity_memory::RemovedVehicle::Cart(removed),
                    );
                }
            })
            .is_none()
        {
            return true;
        }
        if !explodes {
            return true;
        }
        // ExplosionInteraction.TNT: the tnt_explosion_drop_decay rule.
        let interaction =
            self.destroy_type(sunrise_game_data::gamerules::vanilla::TNT_EXPLOSION_DROP_DECAY);
        // MinecartTNT.isPrimed (fuse > -1): a cart that ran into a wall or
        // fell unprimed blows rails up like any blast.
        self.server.combat.0.rail_proof_blast = c.fuse >= 0;
        self.server.combat.0.explosion_in_water = self.bounds_in_water(c.bounds());
        self.server.combat.0.explosion_direct = Some((c.id, "minecraft:tnt_minecart"));
        let _ = self.with_hive_emptying_blast(|g| {
            g.server_explode_from(
                at,
                power,
                interaction,
                false,
                super::super::explosion::ExplosionSource::default()
                    .with_silence(silent)
                    .with_entity(
                        c.id,
                        "minecraft:tnt_minecart",
                        sunrise_sim::health::DirectEntity::Other,
                    ),
            )
        });
        self.server.combat.0.explosion_direct = None;
        self.server.combat.0.explosion_in_water = false;
        self.server.combat.0.rail_proof_blast = false;
        true
    }
}
