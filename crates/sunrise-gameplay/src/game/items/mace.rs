//! Maces (`MaceItem`): a hit after falling more than one and a half blocks is
//! a smash attack. It adds damage for the height fallen (more with Density),
//! stops the fall, cancels its damage and knocks the creatures around the
//! target away. The rest of a weapon hit (`Player.itemAttackInteraction`)
//! is here too: `hurtEnemy`, the post-attack enchantments and `postHurtEnemy`.
use crate::game::ServerGame;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::mace_common::{KNOCKBACK_RADIUS, can_smash, smash_damage};
use sunrise_items::{
    enchant::{EffectContext, Enchantments},
    items::{ItemStack, Items},
};
use sunrise_sim::{
    health::ArmorWeapon,
    mobs::{Damage, DamageKind, Mob, MobEvent, Species},
};

/// `CombatRules.getDamageAfterAbsorb`: the attacking weapon's armor effectiveness.
pub(in crate::game::server_side) fn breach_modifier<'a>(
    weapon: Option<&'a ItemStack>,
    target: &'a str,
    source: DamageKind,
    items: &'a Items,
    enchantments: &'a Enchantments,
) -> impl FnMut(f32, &mut sunrise_core::random::LegacyRandom) -> f32 + use<'a> {
    let ctx = EffectContext {
        this: Some(target),
        attacker: Some("minecraft:player"),
        direct_attacker: Some("minecraft:player"),
        is_direct: true,
        damage_type: Some(match source {
            DamageKind::MaceSmash => "minecraft:mace_smash",
            DamageKind::Spear => "minecraft:spear",
            DamageKind::Thorns => "minecraft:thorns",
            _ => "minecraft:player_attack",
        }),
        items: Some(items),
        ..Default::default()
    };
    move |fraction, random| {
        enchantments.modify_value(
            weapon,
            "minecraft:armor_effectiveness",
            &ctx,
            random,
            fraction,
        )
    }
}

/// `MaceItem.SMASH_ATTACK_HEAVY_THRESHOLD`.
pub(in crate::game) const HEAVY_THRESHOLD: f64 = 5.0;
/// `MaceItem.SMASH_ATTACK_KNOCKBACK_POWER`.
pub(in crate::game) const KNOCKBACK_POWER: f32 = 0.7;

/// `Entity.isAlliedTo` between the player (`player` its UUID) and a mob
/// without teams: `TamableAnimal.considersEntityAsAlly` (a pet it tamed) and
/// `Allay.considersEntityAsAlly` (an allay that likes it).
pub(in crate::game::server_side) fn ally_of_player(m: &Mob, player: Option<[i32; 4]>) -> bool {
    let tamable = matches!(
        m.species(),
        Species::Wolf
            | Species::Cat
            | Species::Parrot
            | Species::Nautilus
            | Species::ZombieNautilus
    );
    tamable && m.extra.tame && m.extra.owner.is_some() && m.extra.owner == player
        || m.species() == Species::Allay && m.extra.tame
}

/// The entity a sweep leaves out: the one the attack hit (`target` of
/// `Player.doSweepAttack`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Spared {
    Nothing,
    Mob(u32),
    Stand(u32),
    /// A player of the open world, by entity id.
    Player(i32),
}

/// `Player.doSweepAttack`'s `1 + ratio * f`, where `f` is the attack damage
/// after `baseDamageScaleFactor` (0.2 + strength^2 * 0.8).
pub(in crate::game::server_side) fn sweep_base_damage(
    ratio: f32,
    attack_damage: f32,
    scale: f32,
) -> f32 {
    1.0 + ratio * attack_damage * (0.2 + scale * scale * 0.8)
}
/// A smash's knockback on the players, put off while a guest's turn holds
/// the other players.
pub(in crate::game) struct SmashPush {
    pub(in crate::game) center: Vec3,
    pub(in crate::game) heavy: f64,
    pub(in crate::game) attacker: Option<u32>,
    pub(in crate::game) team: Option<sunrise_game_data::scoreboard::Team>,
}
/// `MaceItem.getKnockbackPower` and the push of one entity at `at`, `heavy`
/// being 2 after a fall of more than five blocks.
pub(in crate::game::server_side) fn smash_push(
    center: Vec3,
    heavy: f64,
    at: Vec3,
    velocity: &mut Vec3,
    resistance: f64,
) {
    let d = [at.x - center.x, at.y - center.y, at.z - center.z];
    let distance_sqr = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if distance_sqr > KNOCKBACK_RADIUS.powi(2) {
        return;
    }
    let length = distance_sqr.sqrt();
    let power =
        (KNOCKBACK_RADIUS - length) * f64::from(KNOCKBACK_POWER) * heavy * (1.0 - resistance);
    if power <= 0.0 {
        return;
    }
    // Vec3.normalize: a vector shorter than 1e-5 has no direction.
    let (x, z) = if length < f64::from(1.0e-5f32) {
        (0.0, 0.0)
    } else {
        (d[0] / length * power, d[2] / length * power)
    };
    *velocity = Vec3::new(
        velocity.x + x,
        velocity.y + f64::from(KNOCKBACK_POWER),
        velocity.z + z,
    );
}

impl ServerGame<'_> {
    pub(in crate::game::server_side) fn holds_mace(&self) -> bool {
        self.common.inventory[self.common.selected]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:mace")
    }
    /// `MaceItem.canSmashAttack` for the player: falling and not gliding.
    pub(in crate::game::server_side) fn can_smash_attack(&self) -> bool {
        self.common
            .player
            .as_ref()
            .is_some_and(|p| can_smash(p.fall_distance, p.fall_flying))
    }
    /// A held mace about to smash (`MaceItem.getItemDamageSource`: the attack
    /// is `mace_smash` damage).
    pub(in crate::game::server_side) fn smash_attack(&self) -> bool {
        self.holds_mace() && self.can_smash_attack()
    }
    /// The damage source of the player's hit from `from` with the held item.
    pub fn attack_source(&self, from: [f64; 3]) -> Damage {
        let motion = self
            .common
            .player
            .as_ref()
            .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying));
        let name = self
            .common
            .stack_melee_damage_type(self.common.held(), motion, true);
        let mut source = match name.as_str() {
            "minecraft:mace_smash" => Damage::mace(from),
            _ => Damage::player(from),
        };
        if name != "minecraft:mace_smash"
            && name != "minecraft:player_attack"
            && let Some(named) = sunrise_sim::mobs::NamedDamage::registered(
                &self.common.health_resources.damage,
                &name,
            )
        {
            source.kind = sunrise_sim::mobs::DamageKind::Named(named);
        }
        source
    }

    /// `MaceItem.getAttackDamageBonus` against `victim`: 4 per block for the
    /// first three blocks fallen, 2 for the next five, 1 after that, plus
    /// `smash_damage_per_fallen_block` (Density) for every block.
    pub fn smash_damage_bonus(&mut self, victim: &str, victim_id: Option<u32>) -> f32 {
        if !self.smash_attack() {
            return 0.0;
        }
        let fall = self.common.player.as_ref().map_or(0.0, |p| p.fall_distance);
        let held = self.common.inventory[self.common.selected].clone();
        let Some(held) = held else { return 0.0 };
        let held = self.common.item_combat_stack(&held);
        let source = self.common.stack_melee_damage_type(
            Some(&held),
            self.common
                .player
                .as_ref()
                .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying)),
            true,
        );
        let ctx = EffectContext {
            this: Some(victim),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            damage_type: Some(&source),
            direct_motion: self
                .common
                .player
                .as_ref()
                .map(|p| (p.fall_distance, p.fall_flying)),
            ..Default::default()
        };
        let per_block = self.evaluate_weapon_value(
            &held,
            "minecraft:smash_damage_per_fallen_block",
            victim_id,
            ctx,
            0.0,
        );
        smash_damage(fall, per_block)
    }
    /// `source.getEntity()` of a blow as `SulfurCube.knockback` reads it: its eye position
    /// (`getEyePosition`), view direction (`getLookAngle`, `calculateViewVector(xRot, yRot)`)
    /// and feet (`position`). The player is the one acting, a mob one in the list.
    pub(in crate::game::server_side) fn attacker_aim(
        &self,
        who: Option<sunrise_sim::mobs::Who>,
    ) -> Option<sunrise_sim::mobs::Aim> {
        match who? {
            sunrise_sim::mobs::Who::Player => {
                let p = self.common.player.as_ref()?;
                let look =
                    sunrise_world::view_direction::view_vector(self.common.pitch, self.common.yaw);
                Some(sunrise_sim::mobs::Aim {
                    eye: self.common.eye_position()?,
                    look: [look.x, look.y, look.z],
                    feet: [p.position.x, p.position.y, p.position.z],
                })
            }
            sunrise_sim::mobs::Who::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id)?;
                Some(sunrise_sim::mobs::Aim {
                    eye: [m.position.x, m.eye_y(), m.position.z],
                    look: m.look_angle(),
                    feet: [m.position.x, m.position.y, m.position.z],
                })
            }
        }
    }
    /// `DamageSource.getWeaponItem` names the direct entity's weapon, including the
    /// saved weapon of an arrow or trident, never the shooter's current hand.
    pub(in crate::game::server_side) fn player_armor_weapon(
        &self,
        attack: &sunrise_sim::health::Attack,
        source: sunrise_sim::health::HurtSource<'_>,
    ) -> Option<ArmorWeapon> {
        let stack = attack.weapon_of(source.direct)?;
        let enchantments = &server_world_stations!(self).enchantments;
        enchantments
            .has_component(stack, "minecraft:armor_effectiveness")
            .then(|| ArmorWeapon {
                stack: stack.clone(),
                enchantments: enchantments.clone(),
            })
    }
    /// `LivingEntity.hurtServer` of the mob at `index` by the player's hit:
    /// the weapon's `armor_effectiveness` enchantments (Breach) thin its armor.
    /// Returns whether the hurt landed and whether it killed, before death listeners ran.
    pub fn hurt_by_player(&mut self, index: usize, amount: f32, source: Damage) -> (bool, bool) {
        let attack = self.attack_of_who(Some(sunrise_sim::mobs::Who::Player));
        let plan = self.prepare_knockback_credit(
            self.common.mobs[index].id as i32,
            source.attacker,
            &attack,
        );
        let aim = self.attacker_aim(source.attacker);
        let (id, join, old_health) = (
            self.common.mobs[index].id,
            self.common.mobs[index].join,
            self.common.mobs[index].health,
        );
        // The causing entity's aim, for what turns a blow by where it points
        // (`SulfurCube.knockback`).
        let mob_id = self.common.mobs[index].id;
        self.common.mobs[index].creative_attacker = self.common.creative;
        self.common.mobs[index].attacker_aim = aim;
        self.common.mobs[index].dealt_absorbed = 0;
        let decision =
            crate::game::server_side::entity_server::causer_decision!(self, attack.attacker);
        let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
        let hurt = {
            let (victim, lend) = crate::game::server_side::entity_server::victim_and_causer!(
                self,
                index,
                attack.attacker,
                decision
            );
            victim.damage(
                crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    &attack,
                    &mut credit,
                    lend
                ),
                amount,
                source,
            )
        };
        self.common.mobs[index].attacker_aim = None;
        self.common.mobs[index].creative_attacker = false;
        let Some((mut index, hurt)) = self.complete_mob_hurt(index, hurt) else {
            return (false, false);
        };
        if hurt && self.common.mobs[index].has_protected_hurt() {
            self.flush_mob_death_protection(mob_id);
            let Some(i) = self
                .common
                .mob_index
                .find(&self.common.mobs, id)
                .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
            else {
                return (hurt, false);
            };
            index = i;
        }
        // `Guardian.hurtServer`'s spikes answer the attacker inside the hurt,
        // before `super.hurtServer` returns: those hits land here, where
        // `Who::Player` is the player whose blow this is. Left in the outbox,
        // the end-of-tick sweep would deal them with no player in place (a
        // serving thread's players are all guests).
        let raised = std::mem::take(&mut self.common.mobs[index].outbox);
        let (hits, rest): (Vec<_>, Vec<_>) = raised.into_iter().partition(|e| {
            matches!(
                e,
                MobEvent::DamagePlayer { .. } | MobEvent::DamageMob { .. }
            )
        });
        self.common.mobs[index].outbox = rest;
        let killed = hurt && old_health > 0.0 && !self.common.mobs[index].alive();
        let _ = self.handle_mob_events(hits);
        let Some(index) = self
            .common
            .mob_index
            .find(&self.common.mobs, id)
            .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
        else {
            return (hurt, killed);
        };
        // LivingEntity.actuallyHurt: Stats.DAMAGE_DEALT_ABSORBED for the attacking player,
        // awarded here, during that player's own attack (a server's guest has its own
        // statistics only inside `as_guest`).
        let absorbed = std::mem::take(&mut self.common.mobs[index].dealt_absorbed);
        if absorbed > 0 {
            self.award_custom_stat("damage_dealt_absorbed", absorbed);
        }
        let id = self.common.mobs[index].id;
        if hurt && self.common.mobs[index].health != old_health {
            self.mob_damage_event(id);
        }
        if let Some(index) = self
            .common
            .mob_index
            .find(&self.common.mobs, id)
            .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
        {
            // The accepted hit already committed its player credit, including Thorns.
            self.finish_mob_damage_at(index);
            self.settle_mob_hurt(id);
        }
        (hurt, killed)
    }
    /// Takes the absorbed tenths a projectile's hit left on the mobs; the shooter gets them
    /// as `damage_dealt_absorbed` when it is the player (`LivingEntity.actuallyHurt`).
    pub(in crate::game) fn settle_dealt_absorbed(&mut self, by_player: bool) {
        let absorbed: i32 = self
            .common
            .mobs
            .iter_mut()
            .map(|m| std::mem::take(&mut m.dealt_absorbed))
            .fold(0, i32::saturating_add);
        if by_player && absorbed > 0 {
            self.award_custom_stat("damage_dealt_absorbed", absorbed);
        }
    }
    /// `Player.itemAttackInteraction` after the player's hit landed on the mob
    /// at `index`: the item's `hurtEnemy` (the "used" statistic of a weapon),
    /// the post-attack enchantments, then `postHurtEnemy` with the weapon's
    /// wear.
    pub(in crate::game::server_side) fn weapon_hit(&mut self, index: usize, source: Damage) {
        let victim = self.common.mobs[index].id;
        let kind = self.common.mobs[index].kind.id.clone();
        let damage_type = sunrise_sim::health::source_damage_type(source);
        let Some(stack) = self.common.held().cloned() else {
            // An empty hand still meets the victim's own post-attack
            // enchantments (Thorns).
            self.post_player_melee(victim, &kind, damage_type, None);
            return;
        };
        let items = self.common.res.items();
        let weapon = items
            .weapon_damage(&stack)
            .map(|damage| (damage, items.get(&stack.item).map_or(0, |i| i.max_damage)));
        self.native_hurt_enemy_event(&stack, index);
        let smash = self.smash_attack();
        if smash {
            self.mace_hurt_enemy(index);
        }
        if weapon.is_some() {
            self.award_stat("used", &stack.item, 1);
        }
        self.post_player_melee(victim, &kind, damage_type, Some(&stack));
        // MaceItem.postHurtEnemy
        if smash && let Some(p) = self.common.player.as_mut() {
            p.fall_distance = 0.0;
        }
        self.weapon_wear(&stack);
    }
    /// `ItemStack.postHurtEnemy`: `hurtAndBreak(weapon.itemDamagePerAttack)` of
    /// the weapon in the acting hand.
    pub fn weapon_wear(&mut self, stack: &ItemStack) {
        let items = self.common.res.items();
        let weapon = items
            .weapon_damage(stack)
            .map(|damage| (damage, items.get(&stack.item).map_or(0, |i| i.max_damage)));
        if let Some((per_attack, max)) = weapon
            && max > 0
            && !self.common.creative
        {
            self.damage_held(per_attack);
        }
    }
    /// `MaceItem.hurtEnemy` for a smash: the fall stops and its damage is
    /// waived from where the hit landed, the ground or air smash sounds, and
    /// the knockback around the target.
    pub(in crate::game::server_side) fn mace_hurt_enemy(&mut self, index: usize) {
        let target_on_ground = self.common.mobs[index].ground.on_ground;
        if let Some(fall) = self.mace_smash_self(target_on_ground) {
            let target = &self.common.mobs[index];
            self.smash_knockback(target.position, Spared::Mob(target.id), fall);
        }
    }
    /// What a server's guest does after their hit landed on another player:
    /// the weapon's side, the exhaustion and the sprint a knockback stops.
    /// Returns the attacker's motion when a smash set it, which
    /// `MaceItem.hurtEnemy` sends at once (`ClientboundSetEntityMotionPacket`),
    /// not at the end of the tick after gravity (#8246). The mark from before
    /// the hit stays.
    pub(in crate::game::server_side) fn pvp_hit_after(
        &mut self,
        victim: u32,
        victim_on_ground: bool,
        stops_sprint: bool,
        strike: &super::lan_pvp::Strike,
    ) -> Option<Vec3> {
        let p = self.common.player.as_mut()?;
        let marked = std::mem::take(&mut p.health.sync_velocity);
        self.weapon_hit_player(victim, victim_on_ground, strike);
        let p = self.common.player.as_mut()?;
        let sent = std::mem::replace(&mut p.health.sync_velocity, marked).then_some(p.velocity);
        self.pvp_exhaustion();
        if stops_sprint {
            self.common.stop_sprint_after_extra_knockback();
        }
        sent
    }
    /// `Player.itemAttackInteraction` after the player's hit hurt the player
    /// `victim`: the weapon's side of it, as `weapon_hit` does for a mob. A
    /// smash stops the fall and its Wind Burst (a `post_attack` explosion on
    /// the attacker) launches the attacker.
    fn weapon_hit_player(
        &mut self,
        victim: u32,
        victim_on_ground: bool,
        strike: &super::lan_pvp::Strike,
    ) {
        let Some(stack) = strike.weapon.as_ref() else {
            self.post_player_melee(victim, "minecraft:player", &strike.damage_type, None);
            return;
        };
        let smash = self.smash_attack();
        if smash {
            self.mace_smash_self(victim_on_ground);
        }
        if self.common.res.items().weapon_damage(stack).is_some() {
            self.award_stat("used", &stack.item, 1);
        }
        self.post_player_melee(victim, "minecraft:player", &strike.damage_type, Some(stack));
        // MaceItem.postHurtEnemy
        if smash && let Some(p) = self.common.player.as_mut() {
            p.fall_distance = 0.0;
        }
        self.weapon_wear(stack);
    }
    /// `Player.itemAttackInteraction` after the player's hit hurt the armor
    /// stand `id` (a living target) at `center`: as `weapon_hit` does for a
    /// mob, a smash stops the fall and knocks the creatures around the stand
    /// away, and the Wind Burst (a `post_attack` explosion on the attacker)
    /// launches the attacker.
    pub(in crate::game::server_side) fn weapon_hit_stand(
        &mut self,
        id: u32,
        center: Vec3,
        on_ground: bool,
    ) {
        let Some(stack) = self.common.held().cloned() else {
            return;
        };
        if self.smash_attack()
            && let Some(fall) = self.mace_smash_self(on_ground)
        {
            self.smash_knockback(center, Spared::Stand(id), fall);
        }
        if self.common.res.items().weapon_damage(&stack).is_some() {
            self.award_stat("used", &stack.item, 1);
        }
        let damage_type = self.stack_melee_damage_type(
            Some(&stack),
            self.common
                .player
                .as_ref()
                .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying)),
            true,
        );
        self.post_player_melee(id, "minecraft:armor_stand", &damage_type, Some(&stack));
        // MaceItem.postHurtEnemy
        if self.smash_attack()
            && let Some(p) = self.common.player.as_mut()
        {
            p.fall_distance = 0.0;
        }
        self.weapon_wear(&stack);
    }
    /// `MaceItem.knockback`: the dust pillar (level event 2013) under the
    /// target, then every living entity within 3.5 blocks of it but the
    /// player and the target pushed away and up, harder after a fall of more
    /// than five blocks. Spectators, the player's allies (`isAlliedTo`: its
    /// pets and the allays that like it), marker armor stands and creative
    /// flyers are spared; the predicate's own tamed-animal test asks whether
    /// the target owns the animal, which a mob never does.
    pub(in crate::game::server_side) fn smash_knockback(
        &mut self,
        center: Vec3,
        target: Spared,
        fall: f64,
    ) {
        // Entity.getOnPos without a supporting block.
        let on_pos = [
            center.x.floor() as i32,
            (center.y - f64::from(1.0e-5f32)).floor() as i32,
            center.z.floor() as i32,
        ];
        self.server_level_event_at(2013, on_pos, 750);
        let heavy = if fall > HEAVY_THRESHOLD { 2.0 } else { 1.0 };
        let player = Some(self.acting_uuid());
        for m in self.common.mobs.iter_mut().filter(|m| {
            target != Spared::Mob(m.id) && m.alive() && !m.removed && !ally_of_player(m, player)
        }) {
            let resistance = m.knockback_resistance();
            smash_push(center, heavy, m.position, &mut m.velocity, resistance);
        }
        for s in self
            .common
            .armor_stands
            .iter_mut()
            .filter(|s| !s.marker && target != Spared::Stand(s.id))
        {
            smash_push(center, heavy, s.position, &mut s.velocity, 0.0);
        }
        // The host and the LAN guests are living entities too. A guest's turn
        // holds the other players (`as_guest`): the push waits for it to end.
        let push = SmashPush {
            center,
            heavy,
            attacker: self.server.lan_acting,
            team: self.player_team().cloned(),
        };
        if self.server.lan_acting.is_some() {
            self.server.smash_pushes.push(push);
        } else {
            self.smash_push_players(&push);
        }
    }
    /// `MaceItem.knockback` on the players but the attacker: the host unless
    /// it attacked, and every guest but the attacker.
    pub(in crate::game::server_side) fn smash_push_players(&mut self, push: &SmashPush) {
        if push.attacker.is_some() {
            self.smash_push_player(push);
        }
        self.each_guest_hit_except(push.attacker, |g| g.smash_push_player(push));
    }
    /// The pushes guests' smashes put off.
    pub(in crate::game::server_side) fn flush_smash_pushes(&mut self) {
        for push in std::mem::take(&mut self.server.smash_pushes) {
            self.smash_push_players(&push);
        }
    }
    /// `MaceItem.knockback` on the player here (the host, or the guest swapped
    /// in): not a spectator, an ally of the attacker (`isAlliedTo`: the same
    /// team) or a creative player flying.
    pub(in crate::game::server_side) fn smash_push_player(&mut self, push: &SmashPush) {
        if self.common.spectator()
            || sunrise_game_data::scoreboard::teams_allied(push.team.as_ref(), self.player_team())
        {
            return;
        }
        let creative = self.common.creative;
        let Some(p) = self.common.player.as_mut() else {
            return;
        };
        if creative && p.flying {
            return;
        }
        let resistance = p.attribute("minecraft:knockback_resistance").unwrap_or(0.0);
        smash_push(
            push.center,
            push.heavy,
            p.position,
            &mut p.velocity,
            resistance,
        );
    }
    /// `Player.doSweepAttack`'s victims: every other living mob and every
    /// armor stand (but a marker one) in `target`'s box inflated by
    /// (1, 0.25, 1) and under three blocks from the player takes
    /// `getEnchantedDamage(1 + sweeping ratio * attack damage) * strength`,
    /// knocked back 0.4 along the player's yaw. The player's allies and the
    /// target (`spared`) are spared.
    pub fn sweep_damage(&mut self, target: Aabb, spared: Spared, scale: f32) {
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        let attack = self.attack_of(player.entity_id);
        let from = [player.position.x, player.position.y, player.position.z];
        let area = target.inflate(Vec3::new(1.0, 0.25, 1.0));
        let near = |at: Vec3| {
            (0..3)
                .map(|a| [at.x, at.y, at.z][a] - from[a])
                .map(|d| d * d)
                .sum::<f64>()
                < 9.0
        };
        // The enchantment's `sweeping_damage_ratio` attribute modifier (level /
        // (level + 1)) on the attribute's 0 to 1 range.
        let ratio = self
            .common
            .res
            .stations()
            .enchantments
            .attribute_add(
                self.common.inventory[self.common.selected].as_ref(),
                "minecraft:sweeping_damage_ratio",
            )
            .clamp(0.0, 1.0) as f32;
        // `f` of doSweepAttack: the attack damage attribute after
        // baseDamageScaleFactor (a sweep is never a smash or a critical hit).
        let spin = self
            .common
            .player
            .as_ref()
            .map_or(0.0, |p| p.auto_spin_attack_dmg);
        let attribute = if spin > 0.0 {
            spin
        } else {
            self.attack_damage()
        };
        let base = sweep_base_damage(ratio, attribute, scale);
        let (xd, zd) = self.extra_knockback_direction();
        let uuid = Some(self.acting_uuid());
        let allies = self.player_allies();
        // getEntitiesOfClass walks the entity sections (x, then z, then y)
        // under the box, each in the order its entities were added.
        let section = |at: Vec3| {
            (
                (at.x.floor() as i32) >> 4,
                (at.z.floor() as i32) >> 4,
                (at.y.floor() as i32) >> 4,
            )
        };
        let mut victims: Vec<((i32, i32, i32), u32, bool)> = Vec::new();
        for m in &self.common.mobs {
            if spared != Spared::Mob(m.id)
                && m.alive()
                && !m.removed
                && !allies.contains(&m.id)
                && !ally_of_player(m, uuid)
                && m.bounds().intersects(area)
                && near(m.position)
            {
                victims.push((section(m.position), m.id, false));
            }
        }
        for s in &self.common.armor_stands {
            if !s.marker
                && spared != Spared::Stand(s.id)
                && s.bounds().intersects(area)
                && near(s.position)
            {
                victims.push((section(s.position), s.id, true));
            }
        }
        victims.sort_by_key(|&(key, id, _)| (key, id));
        for (_, id, is_stand) in victims {
            if is_stand {
                let Some(i) = self.common.armor_stands.iter().position(|s| s.id == id) else {
                    continue;
                };
                // knockback(0.4F, ..) follows a hurtServer that landed, on a
                // stand that is still there (LivingEntity.knockback).
                if !self.punch_armor_stand(i) {
                    continue;
                }
                let knock = sunrise_sim::health::Knock {
                    strength: f64::from(0.4_f32),
                    x: xd,
                    z: zd,
                };
                let source = mod_api::entity::health::HurtSource {
                    damage_type: "minecraft:player_attack",
                    causing: attack.attacker,
                    direct: attack.attacker,
                    position: None,
                    direct_type: None,
                };
                self.knock_back_armor_stand(id, knock, source, &attack);
                continue;
            }
            let Some(i) = self.common.mobs.iter().position(|m| m.id == id) else {
                continue;
            };
            let kind = self.common.mobs[i].kind.id.clone();
            let kind = format!("minecraft:{}", kind.trim_start_matches("minecraft:"));
            let amount = self.enchanted_damage(&kind, base, Some(id)) * scale;
            let source = self.attack_source(from);
            // hurtServer first; the knockback and post-attack effects follow
            // only when it landed.
            let join = self.common.mobs[i].join;
            let (hurt, _) = self.hurt_by_player(i, amount, source);
            if hurt {
                let Some(i) = self
                    .common
                    .mob_index
                    .find(&self.common.mobs, id)
                    .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
                else {
                    continue;
                };
                let aim = self.attacker_aim(source.attacker);
                let plan = self.prepare_knockback_credit(id as i32, None, &attack);
                let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                self.common.mobs[i].attacker_aim = aim;
                let level = crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    &attack,
                    &mut credit,
                    None
                );
                self.common.mobs[i].knockback(level, 0.4, xd, zd, &source, amount);
                self.common.mobs[i].attacker_aim = None;
                let held = self.common.held().cloned();
                self.post_player_melee(
                    id,
                    &kind,
                    sunrise_sim::health::source_damage_type(source),
                    held.as_ref(),
                );
            }
        }
        // `getEntitiesOfClass(LivingEntity.class)` takes the other players too.
        let mut sweep = self.player_sweep(base, scale);
        sweep.hit_box = area;
        sweep.skip = match spared {
            Spared::Player(id) => Some(id),
            _ => None,
        };
        self.sweep_other_players(sweep);
    }

    /// The attacker's side of `MaceItem.hurtEnemy` for a smash: the fall
    /// stops, its damage is waived from where the hit landed, and the ground
    /// or air smash sound plays. The fall distance it was, if there is a
    /// player.
    pub(in crate::game) fn mace_smash_self(&mut self, target_on_ground: bool) -> Option<f64> {
        let p = self.common.player.as_mut()?;
        p.velocity = Vec3::new(p.velocity.x, f64::from(0.01f32), p.velocity.z);
        // A server player's client is told (ClientboundSetEntityMotionPacket): it moves by
        // itself, so without it a Wind Burst's push would add to the speed of the fall.
        p.health.sync_velocity = true;
        // calculateImpactPosition: an earlier impact below the player stays.
        let impact = match p.impulse.impact_position {
            Some(at) if at.y <= p.position.y => at,
            _ => p.position,
        };
        p.impulse.set_ignore_fall_damage(true, Some(impact));
        let fall = p.fall_distance;
        let feet = [p.position.x, p.position.y, p.position.z];
        let sound = if target_on_ground {
            p.spawn_extra_particles_on_fall = true;
            if fall > HEAVY_THRESHOLD {
                "minecraft:item.mace.smash_ground_heavy"
            } else {
                "minecraft:item.mace.smash_ground"
            }
        } else {
            "minecraft:item.mace.smash_air"
        };
        self.server_play_sound(sound, feet, 1.0, 1.0);
        Some(fall)
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn sweep_damage_is_one_without_the_enchantment() {
        // A fully charged diamond sword (8 attack damage): neighbours take 1.
        assert_eq!(sweep_base_damage(0.0, 8.0, 1.0), 1.0);
        // Sweeping Edge III adds the ratio of the (charged) attack damage.
        assert_eq!(sweep_base_damage(0.75, 8.0, 1.0), 7.0);
        // baseDamageScaleFactor at 0.95 strength.
        let scaled = sweep_base_damage(0.5, 8.0, 0.95);
        assert!((scaled - (1.0 + 0.5 * 8.0 * (0.2 + 0.95 * 0.95 * 0.8))).abs() < 1e-6);
    }
}
