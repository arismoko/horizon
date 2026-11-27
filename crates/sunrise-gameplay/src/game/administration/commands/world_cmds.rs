//! `/recipe`, `/damage`, `/rotate`, `/swing`, `/playsound`, `/stopsound`,
//! `/particle`, `/attribute`, `/ride` and `/spreadplayers`.
use super::{Arg, Context, R, Target, err, wrap_degrees};
use crate::game::living_attributes::AttributeWrite;
use sunrise_core::random::RandomSource;
use sunrise_game_data::text::Text;
use sunrise_level::sim_settings::SOUND_SOURCES;
use sunrise_sim::{
    mobs::{DamageKind, MobEvent, Who, finalize},
    scoreboard_cmd::{Holder, Tracker},
};
use sunrise_world::attribute_map::ValueKind;
use sunrise_world::damage_source::Catalog as DamageCatalog;

/// Queue bookkeeping for synchronous command damage, preserving prior requests
/// and unrelated events. The existing game policy handles each extracted request.
pub(in crate::game) fn take_new_reinforcements(
    events: &mut Vec<MobEvent>,
    queued: usize,
) -> Vec<(u32, Who)> {
    let mut requests = Vec::new();
    let mut index = 0;
    events.retain(|event| {
        let new = index >= queued;
        index += 1;
        if new && let MobEvent::Reinforcements { mob, target } = event {
            requests.push((*mob, *target));
            false
        } else {
            true
        }
    });
    requests
}

pub(in crate::game) fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
pub(in crate::game) fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}
/// `ChatFormatting`-free float text as `Component.translatable` prints a `double`.
pub(in crate::game) fn java_double_text(v: f64) -> Text {
    Text::literal(sunrise_core::java_number::double_to_string(v))
}

impl crate::game::ServerGame<'_> {
    // ---- /recipe -----------------------------------------------------------

    /// Learnable recipes and registered hidden crafting kinds for `/recipe` resolution.
    pub(in crate::game::server_side) fn learnable_recipes(&self) -> Vec<String> {
        let index = &self.stations().recipe_index;
        index
            .ids()
            .filter(|id| {
                !index.special(id)
                    || matches!(
                        self.recipes().recipe(id),
                        Some(sunrise_sim::crafting::Recipe::Mod(_))
                    )
            })
            .cloned()
            .collect()
    }
    pub(in crate::game::server_side) fn cmd_recipe(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let give = id == "recipe.give";
        let recipes: Vec<String> = match ctx.get("recipe") {
            Some(Arg::Resource(r)) => {
                if !self.learnable_recipes().contains(r) {
                    return Err(err("recipe.notFound", &[r]));
                }
                vec![r.clone()]
            }
            _ => self.learnable_recipes(),
        };
        let mut tracker = Tracker::default();
        for &t in &targets {
            let h = holder(self.target_name(t));
            // ServerPlayer.awardRecipes / resetRecipes on the target's own
            // ServerRecipeBook: a guest's is swapped in for the call, and the
            // packets go to its client.
            let n = self
                .for_player(t, |g| g.change_recipes(give, &recipes))
                .unwrap_or(0);
            tracker.track(&h, n);
        }
        let verb = if give { "give" } else { "take" };
        let (message, total) = tracker.message(
            false,
            Some(err(&format!("commands.recipe.{verb}.failed"), &[])),
            |h, total| {
                Text::translate(
                    &format!("commands.recipe.{verb}.success.single"),
                    &[num(total), h.text.clone()],
                )
            },
            |count, total| {
                Text::translate(
                    &format!("commands.recipe.{verb}.success.multiple"),
                    &[num(total), num(count)],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }

    // ---- /damage ----------------------------------------------------------------

    /// Map an explicit damage type to an engine kind only when the kind
    /// always resolves back to that id. In particular, the engine's explosion
    /// factory chooses the type based on the attacker; an explicit registry
    /// type must not. Other registered types keep their id and tags as `Named`.
    pub fn damage_kind(catalog: &DamageCatalog, ty: &str) -> DamageKind {
        sunrise_sim::health::resolve_damage_kind(Some(catalog), ty).unwrap_or(DamageKind::Generic)
    }

    /// Resolve a mod-supplied source against the world's registry, with no default on a miss.
    pub(in crate::game) fn registered_damage_kind(
        catalog: &DamageCatalog,
        ty: &str,
    ) -> Option<DamageKind> {
        catalog.id(ty)?;
        sunrise_sim::health::resolve_damage_kind(Some(catalog), ty)
    }

    /// `DamageCommand.damage`: apply its source and report an accepted hit.
    pub(in crate::game::server_side) fn cmd_damage(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let amount = match ctx.get("amount") {
            Some(Arg::Float(a)) => *a,
            _ => 0.0,
        };
        let ty = match ctx.get("damageType") {
            Some(Arg::Resource(t)) => t.clone(),
            _ => "minecraft:generic".into(),
        };
        let by = match ctx.get("entity") {
            Some(Arg::Entity(_)) => self.entities(ctx, "entity", false)?.first().copied(),
            _ => None,
        };
        let cause = match ctx.get("cause") {
            Some(Arg::Entity(_)) => self.entities(ctx, "cause", false)?.first().copied(),
            _ => None,
        };
        let from = match ctx.get("location") {
            Some(Arg::Coords(_)) => self.position_arg(ctx, "location"),
            _ => by.and_then(|t| self.target_position(t)),
        };
        // Destruction can remove the target before its success message is built.
        let discarded_name = matches!(target, Target::Item(_) | Target::Boat(_) | Target::Cart(_))
            .then(|| self.target_name(target));
        let hurt = self.hurt_target(target, amount, &ty, by, cause.or(by), from);
        if !hurt {
            return Err(err("commands.damage.invulnerable", &[]));
        }
        let name = discarded_name.unwrap_or_else(|| self.target_name(target));
        self.feedback(Text::translate(
            "commands.damage.success",
            &[
                Text::literal(sunrise_core::java_number::float_to_string(amount)),
                name,
            ],
        ));
        Ok(1)
    }

    /// `DamageCommand.damage`'s routing, the engine's one damage entry by
    /// kind: the player and mob hurt paths (the direct and causing entity
    /// and the source position kept, the death message and thorns credited
    /// to the causing one), the item, vehicle, dragon and crystal branches.
    /// False when the kind refuses the damage. `EntityRef::hurt` (#924)
    /// enters here too — there is no second damage pipeline.
    pub fn hurt_target(
        &mut self,
        target: Target,
        amount: f32,
        ty: &str,
        by: Option<Target>,
        cause: Option<Target>,
        from: Option<[f64; 3]>,
    ) -> bool {
        // Resolve the damage registry at the shared command/API boundary before
        // any target-specific path can mutate state or map it to a DamageKind.
        if !amount.is_finite()
            || amount < 0.0
            || self.common.health_resources.damage.id(ty).is_none()
        {
            return false;
        }
        let attacker = cause.and_then(|t| self.network_entity_id(t));
        let attack = self.command_attack(attacker.unwrap_or(0), by);
        self.with_attack(attack, |g| {
            g.hurt_target_from_source(target, amount, ty, by, cause, from)
        })
    }

    /// `DamageCommand.damage`: an explicit direct entity keeps its actual class and weapon.
    pub(in crate::game::server_side) fn command_attack(
        &self,
        attacker: i32,
        direct: Option<Target>,
    ) -> sunrise_sim::health::Attack {
        use sunrise_sim::health::DirectEntity;
        let mut attack = self.attack_of(attacker);
        attack.direct = Some(
            direct
                .and_then(|target| self.network_entity_id(target))
                .unwrap_or(0),
        );
        attack.direct_type = direct.map(|target| self.target_type(target));
        attack.enchantment_direct =
            direct
                .zip(attack.direct)
                .map(|(target, id)| sunrise_sim::health::EnchantmentActor {
                    id,
                    kind: self.target_type(target).into(),
                    motion: None,
                });
        match direct {
            Some(Target::Projectile(id)) => {
                if let Some(shot) = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|shot| shot.id == id)
                {
                    self.set_projectile_attack(&mut attack, shot);
                }
            }
            Some(Target::Trident(id)) => {
                if let Some(shot) = self.common.tridents.iter().find(|shot| shot.id == id) {
                    attack.projectile = Some(id as i32);
                    attack.projectile_kind = DirectEntity::Arrow;
                    attack.projectile_type = Some("minecraft:trident");
                    self.set_projectile_weapon(&mut attack, Some(&shot.stack));
                }
            }
            _ => {}
        }
        attack.direct_kind = Some(Self::target_direct_entity(
            direct,
            &self.common.creatures.projectiles,
        ));
        attack
    }
    /// Classify an explicit `/damage` or `EntityRef::hurt` direct entity target.
    pub fn target_direct_entity(
        direct: Option<Target>,
        projectiles: &[sunrise_sim::projectiles::Projectile],
    ) -> sunrise_sim::health::DirectEntity {
        use sunrise_sim::health::DirectEntity;
        match direct {
            Some(
                Target::Player
                | Target::Guest(_)
                | Target::Mob(_)
                | Target::Dragon(_)
                | Target::Stand(_),
            ) => DirectEntity::Living,
            Some(Target::Projectile(id)) => projectiles.iter().find(|shot| shot.id == id).map_or(
                DirectEntity::Projectile,
                |shot| {
                    sunrise_gameplay_common::game::CommonGame::projectile_direct_entity(&shot.kind)
                },
            ),
            Some(Target::Trident(_)) => DirectEntity::Arrow,
            Some(Target::Rocket(_) | Target::Hook(_)) => DirectEntity::Projectile,
            _ => DirectEntity::Other,
        }
    }

    // ---- /rotate and /swing -----------------------------------------------------------

    // ---- sounds -----------------------------------------------------------------------

    pub(in crate::game::server_side) fn cmd_playsound(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(sound)) = ctx.get("sound").cloned() else {
            return Ok(0);
        };
        let source = ctx
            .literals
            .iter()
            .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l))
            .unwrap_or(0);
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or_else(|| self.source_position());
        let float = |name: &str, default: f32| match ctx.get(name) {
            Some(Arg::Float(v)) => *v,
            _ => default,
        };
        let (volume, pitch, min_volume) = (
            float("volume", 1.0),
            float("pitch", 1.0),
            float("minVolume", 0.0),
        );
        let range = if volume > 1.0 { 16.0 * volume } else { 16.0 };
        // PlaySoundCommand: one seed for every player.
        let seed = self.common.world.random.next_long();
        let here = self.common.dim.id();
        let mut tracker = Tracker::default();
        for t in targets {
            if !matches!(t, Target::Player | Target::Guest(_)) {
                continue;
            }
            // Only players in the source's level hear it.
            if let Target::Guest(conn) = t
                && self.command_guest(conn).is_none_or(|g| g.dimension != here)
            {
                continue;
            }
            let Some(p) = self.target_position(t) else {
                continue;
            };
            let Some((at, v)) = playsound_placement(pos, p, range, volume, min_volume) else {
                continue;
            };
            self.play_sound_to(t, &sound, at, v, pitch, source, seed);
            tracker.mark(&holder(self.target_name(t)));
        }
        let name = Text::literal(sound.clone());
        let (message, total) = tracker.message(
            true,
            Some(err("commands.playsound.failed", &[])),
            |h, _| {
                Text::translate(
                    "commands.playsound.success.single",
                    &[name.clone(), h.text.clone()],
                )
            },
            |c, _| {
                Text::translate(
                    "commands.playsound.success.multiple",
                    &[name.clone(), num(c)],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    /// `ClientboundSoundPacket` to one player: a guest's connection is sent
    /// it; the window's own player plays it. `/playsound` and a native mod's
    /// `PlayerRef::play_sound` both send through here.
    pub(in crate::game::server_side) fn play_sound_to(
        &mut self,
        t: Target,
        sound: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: usize,
        seed: i64,
    ) {
        self.send_player_sound(t, sound, at, volume, pitch, source, seed, false);
    }
    /// Direct packet delivery for a committed `PlayerRef::play_sound`, bypassing `guest_packets`
    /// during a guest turn so packet order matches commit order across all effect kinds.
    pub(in crate::game::server_side) fn deliver_player_sound(
        &mut self,
        t: Target,
        sound: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: usize,
        seed: i64,
    ) {
        self.send_player_sound(t, sound, at, volume, pitch, source, seed, true);
    }
    /// `StopSoundCommand`: each target's connection gets the packet; the
    /// window's own player stops its sounds. `/stopsound` and a native mod's
    /// `PlayerRef::stop_sound` both send through here.
    pub(in crate::game::server_side) fn stop_sound_for(
        &mut self,
        targets: &[Target],
        sound: Option<&str>,
        source: Option<usize>,
    ) {
        self.send_player_stop_sound(targets, sound, source, false);
    }
    /// Direct packet delivery for a committed `PlayerRef::stop_sound`, bypassing `guest_packets`
    /// during a guest turn so packet order matches commit order across all effect kinds.
    pub(in crate::game::server_side) fn deliver_player_stop_sound(
        &mut self,
        t: Target,
        sound: Option<&str>,
        source: Option<usize>,
    ) {
        self.send_player_stop_sound(&[t], sound, source, true);
    }
    pub(in crate::game::server_side) fn cmd_stopsound(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let source = ctx
            .literals
            .iter()
            .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l));
        let sound = match ctx.get("sound") {
            Some(Arg::Resource(s)) => Some(s.clone()),
            _ => None,
        };
        self.stop_sound_for(&targets, sound.as_deref(), source);
        let key = match (source, &sound) {
            (Some(_), Some(_)) => "commands.stopsound.success.source.sound",
            (Some(_), None) => "commands.stopsound.success.source.any",
            (None, Some(_)) => "commands.stopsound.success.sourceless.sound",
            (None, None) => "commands.stopsound.success.sourceless.any",
        };
        let mut args = Vec::new();
        if let Some(s) = &sound {
            args.push(Text::literal(s.clone()));
        }
        if let Some(i) = source {
            args.push(Text::literal(SOUND_SOURCES[i]));
        }
        self.feedback(Text::translate(key, &args));
        Ok(targets.len() as i32)
    }

    // ---- /particle ----------------------------------------------------------------------

    // ---- /attribute ---------------------------------------------------------------------

    /// The attribute id without `minecraft:`, as the attribute map names it.
    pub(in crate::game::server_side) fn attribute_description(attribute: &str) -> Text {
        let short = attribute.trim_start_matches("minecraft:");
        Text::translate(&format!("attribute.name.{short}"), &[])
    }
    /// Read arms follow `AttributeCommand.getAttributeValue` and
    /// `AttributeCommand.getAttributeModifier`: effective values include modeled
    /// transient modifiers; base reads still use the unmodified base.
    /// `AttributeCommand` writes the instance without delivering its dirty
    /// callbacks; the living entity refreshes them at its own lifecycle boundary.
    pub(in crate::game::server_side) fn cmd_attribute(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let Some(Arg::Resource(attribute)) = ctx.get("attribute").cloned() else {
            return Ok(0);
        };
        // An unbounded selector reaches every loaded level, so the target may
        // be in another one than the command's (EntitySelector.findEntities,
        // AttributeCommand acting on the entity itself).
        let reply = if matches!(target, Target::Guest(_)) {
            self.for_player(target, |g| {
                g.attribute_in_level(id, ctx, Target::Player, &attribute)
            })
        } else {
            self.in_target_level(target, |g| {
                g.attribute_in_level(id, ctx, target, &attribute)
            })
        }
        .unwrap_or(Ok(None))?;
        // Command success belongs to the issuer, after the actual target's
        // body/level is restored. Internal system messages still use for_player.
        let Some((value, message)) = reply else {
            return Ok(0);
        };
        self.feedback(message);
        Ok(value)
    }
    /// `/attribute` on a target, with the target's own level in place.
    pub(in crate::game::server_side) fn attribute_in_level(
        &mut self,
        id: &str,
        ctx: &Context,
        target: Target,
        attribute: &str,
    ) -> R<Option<(i32, Text)>> {
        let name = self.target_name(target);
        let description = Self::attribute_description(attribute);
        let short = attribute.trim_start_matches("minecraft:").to_owned();
        let no_attribute = |name: &Text, d: &Text| {
            text_error(
                "commands.attribute.failed.no_attribute",
                &[name.clone(), d.clone()],
            )
        };
        let scale = match ctx.get("scale") {
            Some(Arg::Double(s)) => *s,
            _ => 1.0,
        };
        let value_arg = match ctx.get("value") {
            Some(Arg::Double(v)) => *v,
            _ => 0.0,
        };
        let modifier_id = match ctx.get("id") {
            Some(Arg::Resource(m)) => m.clone(),
            _ => String::new(),
        };
        match target {
            Target::Player => {
                let Some(player) = self.common.player.as_mut() else {
                    return Ok(None);
                };
                let Some(key) = player.attributes.key(attribute) else {
                    return Err(no_attribute(&name, &description));
                };
                // Every arm below starts from an instance or a supplier value,
                // and vanilla's `getAttributeInstance` (modifier add/remove,
                // base set), `getEntityWithAttribute` (the gets) and
                // `resetBaseValue` all throw `no_attribute` when the supplier
                // creates none: a player has no `minecraft:follow_range`.
                if !player
                    .attributes
                    .has_attribute(Some(key.clone()))
                    .unwrap_or(false)
                {
                    return Err(no_attribute(&name, &description));
                }
                let unavailable = |_| no_attribute(&name, &description);
                match id {
                    "attribute.get" | "attribute.base.get" => {
                        let kind = if id == "attribute.get" {
                            ValueKind::Computed
                        } else {
                            ValueKind::Base
                        };
                        let v = player
                            .attributes
                            .value(Some(key), kind)
                            .map_err(unavailable)?;
                        let k = if id == "attribute.get" {
                            "commands.attribute.value.get.success"
                        } else {
                            "commands.attribute.base_value.get.success"
                        };
                        Ok(Some((
                            (v * scale) as i32,
                            Text::translate(k, &[description, name, java_double_text(v)]),
                        )))
                    }
                    "attribute.base.set" => {
                        self.write_attribute(target, attribute, AttributeWrite::Base(value_arg))
                            .ok_or_else(|| no_attribute(&name, &description))?;
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.set.success",
                                &[description, name, java_double_text(value_arg)],
                            ),
                        )))
                    }
                    "attribute.base.reset" => {
                        self.write_attribute(target, attribute, AttributeWrite::ResetBase)
                            .ok_or_else(|| no_attribute(&name, &description))?;
                        let v = self
                            .common
                            .player
                            .as_mut()
                            .and_then(|p| p.attributes.value(Some(key), ValueKind::Base).ok())
                            .unwrap_or(0.0);
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.reset.success",
                                &[description, name, java_double_text(v)],
                            ),
                        )))
                    }
                    "attribute.modifier.add" => {
                        let added = AttributeWrite::add(
                            &modifier_id,
                            value_arg,
                            command_operation(ctx),
                            true,
                            false,
                        )
                        .and_then(|write| self.write_attribute(target, attribute, write))
                        .ok_or_else(|| no_attribute(&name, &description))?;
                        if !added {
                            return Err(text_error(
                                "commands.attribute.failed.modifier_already_present",
                                &[
                                    Text::literal(modifier_id.clone()),
                                    description.clone(),
                                    name.clone(),
                                ],
                            ));
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.add.success",
                                &[Text::literal(modifier_id), description, name],
                            ),
                        )))
                    }
                    "attribute.modifier.remove" => {
                        let removed = self.write_attribute(
                            target,
                            attribute,
                            AttributeWrite::Remove(modifier_id.clone()),
                        );
                        if removed != Some(true) {
                            return Err(text_error(
                                "commands.attribute.failed.no_modifier",
                                &[
                                    description.clone(),
                                    name.clone(),
                                    Text::literal(modifier_id.clone()),
                                ],
                            ));
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.remove.success",
                                &[Text::literal(modifier_id), description, name],
                            ),
                        )))
                    }
                    _ => {
                        // getAttributeModifier: getEntityWithAttribute checks
                        // hasAttribute, then hasModifier and getModifierValue read
                        // the cached instance or the type's supplier. None of
                        // them calls getInstance, so a failed lookup leaves the
                        // attribute map (and the `attributes` list a save packs)
                        // untouched.
                        if !player
                            .attributes
                            .has_attribute(Some(key.clone()))
                            .map_err(unavailable)?
                        {
                            return Err(no_attribute(&name, &description));
                        }
                        let Some(v) = player
                            .attributes
                            .modifier_value(Some(key), &modifier_id)
                            .ok()
                        else {
                            return Err(text_error(
                                "commands.attribute.failed.no_modifier",
                                &[
                                    description.clone(),
                                    name.clone(),
                                    Text::literal(modifier_id.clone()),
                                ],
                            ));
                        };
                        Ok(Some((
                            (v * scale) as i32,
                            Text::translate(
                                "commands.attribute.modifier.value.get.success",
                                &[
                                    Text::literal(modifier_id),
                                    description,
                                    name,
                                    java_double_text(v),
                                ],
                            ),
                        )))
                    }
                }
            }
            Target::Mob(_) | Target::Stand(_)
                if self.sulfur_archetype_attribute(target, attribute)
                    || !(matches!(target, Target::Mob(_))
                        && finalize::command_attr(short.as_str()).is_some()) =>
            {
                let op = ctx
                    .literals
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "add_value".into());
                self.generic_attribute(
                    target,
                    id,
                    attribute,
                    (&name, &description),
                    (value_arg, scale, &modifier_id, &op),
                )
                .map(Some)
            }
            Target::Mob(mob_id) => {
                let Some(m) = self.common.mobs.iter().find(|m| m.id == mob_id) else {
                    return Ok(None);
                };
                // The attributes the mob's gameplay reads live in `finalize`;
                // the command edits that store, never a copy of it.
                let Some(attr) = finalize::command_attr(short.as_str())
                    .filter(|a| finalize::default_value(m, *a).is_some())
                else {
                    return Err(no_attribute(&name, &description));
                };
                let current = if id == "attribute.base.get" {
                    finalize::base_value(m, attr)
                } else {
                    finalize::value(m, attr)
                };
                let no_modifier = text_error(
                    "commands.attribute.failed.no_modifier",
                    &[
                        description.clone(),
                        name.clone(),
                        Text::literal(modifier_id.clone()),
                    ],
                );
                match id {
                    "attribute.get" | "attribute.base.get" => {
                        let k = if id == "attribute.get" {
                            "commands.attribute.value.get.success"
                        } else {
                            "commands.attribute.base_value.get.success"
                        };
                        Ok(Some((
                            (current * scale) as i32,
                            Text::translate(k, &[description, name, java_double_text(current)]),
                        )))
                    }
                    "attribute.base.set" => {
                        // AttributeInstance keeps the raw base and clamps only the
                        // effective value.
                        self.write_attribute(target, attribute, AttributeWrite::Base(value_arg));
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.set.success",
                                &[description, name, java_double_text(value_arg)],
                            ),
                        )))
                    }
                    "attribute.base.reset" => {
                        self.write_attribute(target, attribute, AttributeWrite::ResetBase);
                        let v = self
                            .common
                            .mobs
                            .iter()
                            .find(|m| m.id == mob_id)
                            .map_or(0.0, |m| finalize::base_value(m, attr));
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.base_value.reset.success",
                                &[description, name, java_double_text(v)],
                            ),
                        )))
                    }
                    "attribute.modifier.add" => {
                        let added = AttributeWrite::add(
                            &modifier_id,
                            value_arg,
                            command_operation(ctx),
                            true,
                            false,
                        )
                        .and_then(|write| self.write_attribute(target, attribute, write));
                        if added != Some(true) {
                            return Err(text_error(
                                "commands.attribute.failed.modifier_already_present",
                                &[Text::literal(modifier_id.clone()), description, name],
                            ));
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.add.success",
                                &[Text::literal(modifier_id.clone()), description, name],
                            ),
                        )))
                    }
                    "attribute.modifier.remove" => {
                        let removed = self.write_attribute(
                            target,
                            attribute,
                            AttributeWrite::Remove(modifier_id.clone()),
                        );
                        if removed != Some(true) {
                            return Err(no_modifier);
                        }
                        Ok(Some((
                            1,
                            Text::translate(
                                "commands.attribute.modifier.remove.success",
                                &[Text::literal(modifier_id.clone()), description, name],
                            ),
                        )))
                    }
                    _ => {
                        // `AttributeInstance.getModifier` finds a permanent or a
                        // transient modifier: a mod's transient one, then the
                        // modeled baby modifier, which is transient and absent
                        // from the saved store.
                        let amount = finalize::modifier_amount(m, attr, &modifier_id)
                            .or_else(|| finalize::transient_modifier_amount(m, attr, &modifier_id))
                            .or_else(|| {
                                finalize::transient(m, attr)
                                    .filter(|_| modifier_id == "minecraft:baby")
                                    .map(|(amount, _)| amount)
                            });
                        let Some(v) = amount else {
                            return Err(no_modifier);
                        };
                        Ok(Some((
                            (v * scale) as i32,
                            Text::translate(
                                "commands.attribute.modifier.value.get.success",
                                &[
                                    Text::literal(modifier_id.clone()),
                                    description,
                                    name,
                                    java_double_text(v),
                                ],
                            ),
                        )))
                    }
                }
            }
            _ => Err(text_error(
                "commands.attribute.failed.entity",
                std::slice::from_ref(&name),
            )),
        }
    }

    /// A sulfur cube's body archetype attributes live on its own map, beside
    /// the archetype's transient modifiers.
    pub(in crate::game::server_side) fn sulfur_archetype_attribute(
        &self,
        t: Target,
        attribute: &str,
    ) -> bool {
        let Target::Mob(id) = t else {
            return false;
        };
        sunrise_sim::mobs::sulfur_cube::ARCHETYPE_ATTRIBUTES.contains(&attribute)
            && self
                .common
                .mobs
                .iter()
                .any(|m| m.id == id && m.species() == sunrise_sim::mobs::Species::SulfurCube)
    }
    /// AttributeCommand on a living entity through its generic attribute map.
    pub(in crate::game::server_side) fn generic_attribute(
        &mut self,
        target: Target,
        id: &str,
        attribute: &str,
        (name, description): (&Text, &Text),
        (value_arg, scale, modifier_id, op): (f64, f64, &str, &str),
    ) -> R<(i32, Text)> {
        let no_attribute = || {
            text_error(
                "commands.attribute.failed.no_attribute",
                &[name.clone(), description.clone()],
            )
        };
        let no_modifier = || {
            text_error(
                "commands.attribute.failed.no_modifier",
                &[
                    description.clone(),
                    name.clone(),
                    Text::literal(modifier_id),
                ],
            )
        };
        let catalog = self.common.motion.attributes.clone();
        let key = catalog.id(attribute).ok_or_else(no_attribute)?;
        let map = self.generic_attributes(target).ok_or_else(no_attribute)?;
        if !map.has(key) {
            return Err(no_attribute());
        }
        let (message, args, result) = match id {
            "attribute.get" | "attribute.base.get" => {
                let v = if id == "attribute.get" {
                    map.value(key).map_err(|_| no_attribute())?
                } else {
                    map.base_value(key).map_err(|_| no_attribute())?
                };
                let k = if id == "attribute.get" {
                    "commands.attribute.value.get.success"
                } else {
                    "commands.attribute.base_value.get.success"
                };
                (
                    k,
                    vec![description.clone(), name.clone(), java_double_text(v)],
                    (v * scale) as i32,
                )
            }
            "attribute.base.set" => {
                self.write_attribute(target, attribute, AttributeWrite::Base(value_arg))
                    .ok_or_else(no_attribute)?;
                (
                    "commands.attribute.base_value.set.success",
                    vec![
                        description.clone(),
                        name.clone(),
                        java_double_text(value_arg),
                    ],
                    1,
                )
            }
            "attribute.base.reset" => {
                self.write_attribute(target, attribute, AttributeWrite::ResetBase)
                    .ok_or_else(no_attribute)?;
                let v = self
                    .generic_attributes(target)
                    .and_then(|map| map.base_value(key).ok())
                    .unwrap_or(0.0);
                (
                    "commands.attribute.base_value.reset.success",
                    vec![description.clone(), name.clone(), java_double_text(v)],
                    1,
                )
            }
            "attribute.modifier.add" => {
                let write =
                    AttributeWrite::add(modifier_id, value_arg, operation_named(op), true, false)
                        .ok_or_else(no_attribute)?;
                if !self
                    .write_attribute(target, attribute, write)
                    .ok_or_else(no_attribute)?
                {
                    return Err(text_error(
                        "commands.attribute.failed.modifier_already_present",
                        &[
                            Text::literal(modifier_id),
                            description.clone(),
                            name.clone(),
                        ],
                    ));
                }
                (
                    "commands.attribute.modifier.add.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                    ],
                    1,
                )
            }
            "attribute.modifier.remove" => {
                let write = AttributeWrite::Remove(modifier_id.to_owned());
                if self.write_attribute(target, attribute, write) != Some(true) {
                    return Err(no_modifier());
                }
                (
                    "commands.attribute.modifier.remove.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                    ],
                    1,
                )
            }
            _ => {
                let v = map
                    .modifier_value(key, modifier_id)
                    .map_err(|_| no_modifier())?;
                (
                    "commands.attribute.modifier.value.get.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                        java_double_text(v),
                    ],
                    (v * scale) as i32,
                )
            }
        };
        Ok((result, Text::translate(message, &args)))
    }

    // ---- /ride ----------------------------------------------------------------------------

    pub(in crate::game::server_side) fn cmd_ride(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let name = self.target_name(target);
        // A server player other than the one in place (the console's, a function's or another
        // player's /ride): their riding state is their own, so the command runs as them, as
        // `native_mount` does. In vanilla every ServerPlayer rides alike.
        if let Target::Guest(_) = target {
            return self.ride_guest(id, ctx, target, name);
        }
        if id == "ride.dismount" {
            let Some(v) = self.ride_vehicle(target) else {
                return Err(err("commands.ride.not_riding", &[&name.plain()]));
            };
            let vehicle_name = self.target_name(v);
            match target {
                Target::Player => self.stop_riding(),
                Target::Mob(rider) => self.dismount_mob(rider),
                Target::Bolt(rider) => {
                    if let Some(i) = self.common.bolts.iter().position(|b| b.id == rider) {
                        self.common.bolts.update(i, |b| b.mount = None);
                    }
                }
                _ => {}
            }
            self.feedback(Text::translate(
                "commands.ride.dismount.success",
                &[name, vehicle_name],
            ));
            return Ok(1);
        }
        let vehicles = self.entities(ctx, "vehicle", false)?;
        let Some(vehicle) = vehicles.first().copied() else {
            return Ok(0);
        };
        let vehicle_name = self.target_name(vehicle);
        self.ride_mount(target, vehicle)?;
        self.feedback(Text::translate(
            "commands.ride.mount.success",
            &[name, vehicle_name],
        ));
        Ok(1)
    }

    /// `cmd_ride` for a server player not in place (`Target::Guest`): `RideCommand.dismount` and
    /// `RideCommand.mount` run as that player, through `for_player`.
    pub(in crate::game::server_side) fn ride_guest(
        &mut self,
        id: &str,
        ctx: &Context,
        target: Target,
        name: Text,
    ) -> R<i32> {
        if id == "ride.dismount" {
            let not_riding = || err("commands.ride.not_riding", &[&name.plain()]);
            let vehicle_name = self
                .for_player(target, |g| {
                    let vehicle = g.ride_vehicle(Target::Player)?;
                    let vehicle_name = g.target_name(vehicle);
                    g.stop_riding();
                    Some(vehicle_name)
                })
                .flatten()
                .ok_or_else(not_riding)?;
            self.feedback(Text::translate(
                "commands.ride.dismount.success",
                &[name, vehicle_name],
            ));
            return Ok(1);
        }
        let vehicles = self.entities(ctx, "vehicle", false)?;
        let Some(vehicle) = vehicles.first().copied() else {
            return Ok(0);
        };
        let vehicle_name = self.target_name(vehicle);
        self.for_player(target, |g| g.ride_mount(Target::Player, vehicle))
            .unwrap_or_else(|| {
                Err(err(
                    "commands.ride.mount.failure.generic",
                    &[&name.plain(), &vehicle_name.plain()],
                ))
            })?;
        self.feedback(Text::translate(
            "commands.ride.mount.success",
            &[name, vehicle_name],
        ));
        Ok(1)
    }

    /// `RideCommand.mount` once the targets are known: `target` (the player
    /// in place or a mob) rides `vehicle`, as `startRiding(vehicle, true,
    /// true)` forces it (also for `native_mount`).
    pub fn ride_mount(&mut self, target: Target, vehicle: Target) -> R<()> {
        let name = self.target_name(target);
        let vehicle_name = self.target_name(vehicle);
        if let Some(current) = self.ride_vehicle(target) {
            let current = self.target_name(current);
            return Err(err(
                "commands.ride.already_riding",
                &[&name.plain(), &current.plain()],
            ));
        }
        // RideCommand.mount: `vehicle.getType() == EntityType.PLAYER`, any server player.
        if matches!(vehicle, Target::Player | Target::Guest(_)) {
            return Err(err("commands.ride.mount.failure.cant_ride_players", &[]));
        }
        // getSelfAndPassengers: the vehicle is the target or rides it.
        let mut up = Some(vehicle);
        while let Some(v) = up {
            if v == target {
                return Err(err("commands.ride.mount.failure.loop", &[]));
            }
            up = self.ride_vehicle(v);
        }
        if !self.start_riding_target(target, vehicle, true) {
            return Err(err(
                "commands.ride.mount.failure.generic",
                &[&name.plain(), &vehicle_name.plain()],
            ));
        }
        Ok(())
    }

    /// LivingEntity.stopRiding for a mob: off its vehicle
    /// (Entity.removePassenger), then to where the vehicle lets it out
    /// (LivingEntity.dismountVehicle).
    pub fn dismount_mob(&mut self, rider: u32) {
        self.dismount_mob_with_cart(rider, None, None, false);
    }

    pub(in crate::game::server_side) fn dismount_mob_from_cart(
        &mut self,
        rider: u32,
        cart: &super::super::minecarts::Minecart,
    ) {
        self.dismount_mob_with_cart(rider, Some(cart), None, false);
    }

    pub(in crate::game::server_side) fn dismount_mob_reentrant(&mut self, rider: u32) {
        self.dismount_mob_with_cart(rider, None, None, true);
    }

    pub(in crate::game::server_side) fn dismount_mob_from_removed_vehicle(
        &mut self,
        rider: u32,
        y: f64,
    ) {
        self.dismount_mob_with_cart(rider, None, Some(y), false);
    }

    pub(in crate::game::server_side) fn dismount_mob_with_cart(
        &mut self,
        rider: u32,
        cart: Option<&super::super::minecarts::Minecart>,
        removed_y: Option<f64>,
        notified: bool,
    ) {
        let Some(mob) = self.common.mob_object(rider) else {
            return;
        };
        let removing_rider = mob.removed;
        let Some(v) = mob.extra.mount else {
            return;
        };
        if cart.is_some_and(|cart| cart.id != v) {
            return;
        }
        if !notified && !self.native_mob_may_ride(rider, v, false) {
            return;
        }
        // A callback can remove or remount the rider. Do not overwrite its decision.
        if self
            .common
            .mob_object(rider)
            .is_none_or(|mob| mob.removed != removing_rider || mob.extra.mount != Some(v))
        {
            return;
        }
        if let Some(mob) = self.mob_object_mut(rider) {
            mob.extra.mount = None;
        }
        let remaining = self
            .mob_objects()
            .filter(|m| m.extra.mount == Some(v))
            .min_by_key(|m| (m.extra.vehicle_order, m.id))
            .map(|m| m.id);
        if let Some(vehicle) = self.mob_object_mut(v) {
            vehicle.extra.rider = remaining;
        }
        // LivingEntity.dismountVehicle leaves a removed rider at its own position.
        if removing_rider {
            if let Some(mob) = self.mob_object_mut(rider) {
                mob.extra.passenger = false;
            }
            self.release_removed_vehicle(v);
            return;
        }
        let at = if let Some(y) = removed_y {
            self.common.mob_object(rider).map(|mob| {
                self.common.removed_vehicle_dismount_point(
                    mob.position,
                    f64::from(mob.width()),
                    f64::from(mob.height()),
                    y,
                )
            })
        } else {
            match cart.filter(|cart| cart.id == v) {
                Some(cart) => self
                    .common
                    .mob_object(rider)
                    .and_then(|rider| self.cart_dismount_point_from(cart, rider)),
                None => self.mob_dismount_point(v, rider),
            }
        };
        let Some(m) = self.mob_object_mut(rider) else {
            return;
        };
        let at = at.unwrap_or(m.position);
        m.extra.passenger = false;
        m.extra.boarding_cooldown = super::super::boat_passengers::BOARDING_COOLDOWN;
        sunrise_sim::mobs::living::teleports::snap_to(m, at);
        self.snap_mob_passengers(rider);
        self.release_removed_vehicle(v);
    }
    /// Where `dismount_mob` sets object `rider` down off the vehicle `v`: None when
    /// `v` is no boat, cart or mob.
    pub(in crate::game::server_side) fn mob_dismount_point(
        &self,
        v: u32,
        rider: u32,
    ) -> Option<sunrise_core::geometry::Vec3> {
        let mob = self.common.mob_object(rider)?;
        if let Some(vehicle) = self.common.mob_object(v).filter(|vehicle| vehicle.removed) {
            return Some(self.common.removed_vehicle_dismount_point(
                mob.position,
                f64::from(mob.width()),
                f64::from(mob.height()),
                vehicle.position.y,
            ));
        }
        let removed_y = self
            .common
            .persist
            .removed_vehicles
            .get(&v)
            .map(|vehicle| vehicle.position().y)
            .or_else(|| {
                self.common
                    .persist
                    .removals
                    .iter()
                    .chain(self.common.persist.destroying_vehicles.iter())
                    .find_map(|target| match *target {
                        Target::Boat(id) if id == v => self
                            .common
                            .boats
                            .iter()
                            .find(|b| b.id == v)
                            .map(|b| b.position.y),
                        Target::Cart(id) if id == v => self
                            .common
                            .carts
                            .iter()
                            .find(|c| c.id == v)
                            .map(|c| c.position.y),
                        _ => None,
                    })
            });
        if let Some(y) = removed_y {
            return Some(self.common.removed_vehicle_dismount_point(
                mob.position,
                f64::from(mob.width()),
                f64::from(mob.height()),
                y,
            ));
        }
        let (width, yaw) = (f64::from(mob.width()), mob.yaw);
        if let Some(b) = self.boat_object(v) {
            return Some(self.boat_dismount_target(b, width, yaw));
        }
        self.horse_dismount_point(v, rider)
            .or_else(|| self.cart_dismount_point(v, rider))
            .or_else(|| {
                // Entity.getDismountLocationForPassenger: on top of the vehicle.
                let vehicle = self.common.mob_object(v)?;
                Some(super::super::mounts::on_top(vehicle))
            })
    }

    // ---- /spreadplayers -----------------------------------------------------------------------

    /// `SpreadPlayersCommand.Position.getSpawnY`.
    pub(in crate::game::server_side) fn spread_spawn_y(&self, p: [f64; 2], max_y: i32) -> i32 {
        let (x, z) = (p[0].floor() as i32, p[1].floor() as i32);
        let air = |y: i32| {
            let name: &str = &self
                .common
                .res
                .facts()
                .get(self.common.world.block(x, y, z))
                .name;
            matches!(
                name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        };
        let mut y = max_y + 1;
        let mut air2 = air(y);
        y -= 1;
        let mut air1 = air(y);
        while y > self.common.world.bottom_y() {
            y -= 1;
            let current = air(y);
            if !current && air1 && air2 {
                return y + 1;
            }
            air2 = air1;
            air1 = current;
        }
        max_y + 1
    }
    /// `SpreadPlayersCommand.Position.isSafe`.
    pub(in crate::game::server_side) fn spread_safe(&self, p: [f64; 2], max_y: i32) -> bool {
        let y = self.spread_spawn_y(p, max_y) - 1;
        let (x, z) = (p[0].floor() as i32, p[1].floor() as i32);
        let name = self
            .common
            .res
            .facts()
            .get(self.common.world.block(x, y, z))
            .name
            .clone();
        let liquid = matches!(&*name, "minecraft:water" | "minecraft:lava");
        let teleportable = self
            .common
            .res
            .items()
            .block_tags
            .get("entities_can_teleport_to")
            .or_else(|| {
                self.common
                    .res
                    .items()
                    .block_tags
                    .get("minecraft:entities_can_teleport_to")
            })
            .is_some_and(|set| {
                set.contains(&*name) || set.contains(name.trim_start_matches("minecraft:"))
            });
        y < max_y && !liquid && teleportable
    }
}

/// A `CommandError` whose message is a translation with component arguments.
pub(in crate::game) fn text_error(key: &str, args: &[Text]) -> sunrise_command::CommandError {
    sunrise_command::CommandError::new(Text::translate(key, args))
}

/// `PlaySoundCommand.playSound`: the point and the volume one target hears the
/// sound at. A target past `range` hears nothing unless `minVolume` is above
/// zero, and then the point moves two blocks from the target towards the sound
/// and the volume is `minVolume`; a target exactly at `range` still hears it
/// (the command's test is `>`, unlike `PlayerList.broadcast`'s `<`).
pub(in crate::game) fn playsound_placement(
    pos: [f64; 3],
    target: [f64; 3],
    range: f32,
    volume: f32,
    min_volume: f32,
) -> Option<([f64; 3], f32)> {
    let d = [pos[0] - target[0], pos[1] - target[1], pos[2] - target[2]];
    let dist2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    // PlaySoundCommand compares against Mth.square(float range): the square
    // rounds in float and only then widens, so a point between that square and
    // the double square is out of range in vanilla.
    if dist2 <= f64::from(range * range) {
        return Some((pos, volume));
    }
    if min_volume <= 0.0 {
        return None;
    }
    let dist = dist2.sqrt();
    Some((
        [
            target[0] + d[0] / dist * 2.0,
            target[1] + d[1] / dist * 2.0,
            target[2] + d[2] / dist * 2.0,
        ],
        min_volume,
    ))
}

/// `Entity.lookAt`: yaw and pitch from `from` towards `to`, the yaw wrapped
/// to -180..180 as `Mth.wrapDegrees` does.
pub(in crate::game) fn look_at(from: [f64; 3], to: [f64; 3]) -> [f32; 2] {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    // `Entity.lookAt` turns radians to degrees with the float constant
    // `57.2957763671875`, not `Math.toDegrees`.
    let pitch = wrap_degrees((-(dy.atan2(horizontal) * 57.295_776_367_187_5)) as f32);
    let yaw = (dz.atan2(dx) * 57.295_776_367_187_5) as f32 - 90.0;
    [wrap_degrees(yaw), pitch]
}

/// `AttributeModifier.Operation` by its name in `/attribute … modifier add`.
pub(in crate::game) fn operation_named(op: &str) -> sunrise_world::attributes::Operation {
    use sunrise_world::attributes::Operation;
    match op {
        "add_multiplied_base" => Operation::AddMultipliedBase,
        "add_multiplied_total" => Operation::AddMultipliedTotal,
        _ => Operation::AddValue,
    }
}

/// The operation `/attribute … modifier add` names last.
pub(in crate::game) fn command_operation(ctx: &Context) -> sunrise_world::attributes::Operation {
    operation_named(ctx.literals.last().map_or("add_value", String::as_str))
}

#[cfg(test)]
pub(in crate::game) mod allay_owner_damage_tests {}

#[cfg(any(test, feature = "test-support"))]
pub mod datapack_damage_tests {
    use sunrise_world::loot::PackOverlay;

    /// #2647: a damage type a world's datapack declares is in the world's frozen
    /// registry, and `/damage` with it hurts a mob, as `DamageCommand.damage`
    /// does with the `Holder` it resolved there. Every hurt path reads the
    /// world's registry, not the startup catalog, which holds no pack.
    /// The world's registry with the `dmgtest` pack: `zap` untagged, `boom` in
    /// `#is_explosion` and `#no_knockback`, `rush` in `#bypasses_cooldown`, `calm` in
    /// `#no_anger`.
    pub fn with_pack(game: &mut crate::game::ServerGame<'_>) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let ty = |name: &str| {
            (
                format!("damage_type/dmgtest:{name}"),
                vec![
                    format!(r#"{{"message_id":"{name}","scaling":"never","exhaustion":0.1}}"#)
                        .into_bytes(),
                ],
            )
        };
        let tag = |tag: &str, values: &[&str]| {
            let values: Vec<String> = values.iter().map(|v| format!("\"dmgtest:{v}\"")).collect();
            (
                format!("tags/minecraft:damage_type/{tag}"),
                vec![format!(r#"{{"values":[{}]}}"#, values.join(",")).into_bytes()],
            )
        };
        let files = vec![
            ty("zap"),
            ty("boom"),
            ty("rush"),
            ty("calm"),
            tag("is_explosion", &["boom"]),
            tag("no_knockback", &["boom"]),
            tag("bypasses_cooldown", &["rush"]),
            tag("no_anger", &["calm"]),
        ];
        game.health_resources = game
            .motion
            .health
            .with_packs(&root, &PackOverlay::new(&files), false)
            .unwrap();
        assert!(game.motion.health.damage.id("dmgtest:zap").is_none());
    }

    #[allow(unused_imports)]
pub(in crate::game) use crate::game::server_side::commands_server::world_cmds::datapack_damage_tests::pig;

    #[allow(unused_imports)]
#[cfg(any())]
pub(in crate::game) use crate::game::server_side::commands_server::world_cmds::datapack_damage_tests::run_a_datapack_explosion_type_removes_a_crystal_quietly_and_hurts_the_dragon;
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{look_at, playsound_placement, take_new_reinforcements};
    use sunrise_sim::mobs::{MobEvent, Who};

    #[test]
    fn command_reinforcements_leave_prior_requests_and_other_events_queued() {
        let request = |mob| MobEvent::Reinforcements {
            mob,
            target: Who::Player,
        };
        let mut events = vec![request(1), MobEvent::Sound("before", [0.; 3], 1., 1.)];
        let queued = events.len();
        events.extend([
            request(2),
            MobEvent::Sound("after", [0.; 3], 1., 1.),
            request(3),
        ]);
        assert_eq!(
            take_new_reinforcements(&mut events, queued),
            [(2, Who::Player), (3, Who::Player)]
        );
        assert!(matches!(
            events.as_slice(),
            [
                MobEvent::Reinforcements { mob: 1, .. },
                MobEvent::Sound("before", ..),
                MobEvent::Sound("after", ..)
            ]
        ));
        assert!(take_new_reinforcements(&mut events, queued).is_empty());
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn command_reinforcements_tolerate_a_damage_handler_clearing_its_outbox() {
        let mut events = vec![MobEvent::Sound("after", [0.; 3], 1., 1.)];
        assert!(take_new_reinforcements(&mut events, 4).is_empty());
        assert_eq!(events.len(), 1);
    }

    fn assert_angle(got: f32, want: f32) {
        assert!((got - want).abs() < 1.0e-4, "{got} is not {want}");
    }

    /// `Entity.lookAt` wraps the yaw with `Mth.wrapDegrees`, so a target to
    /// the north-west turns the entity to 135 degrees, not to -225, and a
    /// target due north leaves -180 rather than 180.
    #[test]
    fn a_facing_target_wraps_the_yaw_to_180() {
        let at = [0.0, 64.0, 0.0];
        assert_angle(look_at(at, [-1.0, 64.0, -1.0])[0], 135.0);
        assert_angle(look_at(at, [0.0, 64.0, -1.0])[0], -180.0);
        assert_angle(look_at(at, [1.0, 64.0, 0.0])[0], -90.0);
        assert_angle(look_at(at, [0.0, 64.0, 1.0])[0], 0.0);
        assert_angle(look_at(at, [-1.0, 64.0, 0.0])[0], 90.0);
        // The pitch is the elevation, unchanged; its own wrap is a no-op.
        assert_angle(look_at(at, [0.0, 65.0, 1.0])[1], -45.0);
    }

    /// `PlaySoundCommand.playSound`: inside the range the point and volume are
    /// the command's own, exactly on it still is, past it there is nothing
    /// without a minimum volume, and with one the point is two blocks from the
    /// target towards the sound and the volume is that minimum.
    #[test]
    fn a_target_past_the_range_hears_it_two_blocks_away() {
        let pos = [0.0, 64.0, 0.0];
        let near = [0.0, 64.0, 15.0];
        assert_eq!(
            playsound_placement(pos, near, 16.0, 3.0, 0.0),
            Some((pos, 3.0))
        );
        let on_the_edge = [16.0, 64.0, 0.0];
        assert_eq!(
            playsound_placement(pos, on_the_edge, 16.0, 3.0, 0.0),
            Some((pos, 3.0))
        );
        let far = [0.0, 64.0, 32.0];
        assert_eq!(playsound_placement(pos, far, 16.0, 3.0, 0.0), None);
        assert_eq!(
            playsound_placement(pos, far, 16.0, 3.0, 0.5),
            Some(([0.0, 64.0, 30.0], 0.5))
        );
    }

    /// The range is squared as a float, as `Mth.square(getRange(volume))` is:
    /// a point between that square and the double one is past the range in
    /// vanilla, so it gets the `minVolume` placement rather than full volume.
    #[test]
    fn the_range_square_rounds_as_a_float() {
        let volume = 1.1f32;
        let range = 16.0 * volume;
        let float_square = f64::from(range * range);
        let double_square = f64::from(range) * f64::from(range);
        assert!(float_square < double_square);
        let dist2 = (float_square + double_square) / 2.0;
        let pos = [0.0, 64.0, 0.0];
        let target = [dist2.sqrt(), 64.0, 0.0];
        assert_eq!(
            playsound_placement(pos, target, range, volume, 0.0),
            None,
            "past the float range"
        );
        let moved = playsound_placement(pos, target, range, volume, 0.7);
        let Some((point, heard)) = moved else {
            panic!("minVolume should place it");
        };
        assert_eq!(heard, 0.7);
        assert!((point[0] - (target[0] - 2.0)).abs() < 1e-9, "{point:?}");
        assert_eq!(point[1..], [64.0, 0.0]);
    }
}

impl crate::game::ServerGame<'_> {
    /// `PoiTypes`: the point of interest type of a block state.
    pub(in crate::game) fn poi_type(
        &self,
        state: sunrise_world::registry::BlockStateId,
    ) -> Option<&str> {
        let table = &self.stations().professions;
        let state = self.common.pack.assets.registry.state(state)?;
        table.pois.name(table.pois.classify(state)?)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
