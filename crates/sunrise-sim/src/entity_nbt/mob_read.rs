// Included in entity_nbt.rs to retain its private scope.
fn mob_from_nbt_at(
    tag: &Tag,
    kind: Arc<MobType>,
    id: u32,
    registry: Option<&BlockRegistry>,
    random: sunrise_core::random::LegacyRandom,
    position: Vec3,
    authoritative: bool,
) -> Mob {
    let [yaw, pitch] = rotation(tag);
    let mut m = if authoritative {
        Mob::with_random(id, kind, position, yaw, random)
    } else {
        Mob::remote_with_random(id, kind, position, yaw, random)
    };
    m.velocity = motion(tag);
    m.head_pitch = pitch;
    // LivingEntity.readAdditionalSaveData: `setSharedFlag(7, getBooleanOr(FallFlying, false))`.
    m.extra.fall_flying = flag(tag, "FallFlying");
    read_mob_home(&mut m, tag);
    m.ground.on_ground = flag(tag, "OnGround");
    m.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    // AbstractCubeMob.readAdditionalSaveData sizes before the living load, so saved bases win.
    if matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    ) {
        crate::mobs::cube::set_size(
            &mut m,
            (number(tag.get("Size")).unwrap_or(0) as i32).wrapping_add(1),
        );
    }
    // LivingEntity.readAdditionalSaveData: the saved attribute bases, then
    // the health, the max health when there is none.
    if let Some(Tag::List(attrs)) = tag.get("attributes") {
        for a in attrs {
            let (Some(id), Some(v)) = (text(a.get("id")), float(a.get("base"))) else {
                continue;
            };
            let v = if crate::living_attributes::own_unbounded_base(&m.kind.id, &id) {
                v
            } else {
                m.kind.attribute_bounds.loaded_base(&id, v)
            };
            // A mount's rolled health and speed stay attribute bases, which the server syncs.
            let equine = crate::mobs::equine::is_equine(m.species());
            match id.as_str() {
                "minecraft:max_health" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MaxHealth,
                    v,
                ),
                "minecraft:movement_speed" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MovementSpeed,
                    v,
                ),
                // A saved base unlike the current one (type default or cube size) becomes the base.
                "minecraft:max_health"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::MaxHealth,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::MaxHealth,
                        v,
                    )
                }
                "minecraft:max_health" => {
                    m.attrs.max_health = m
                        .kind
                        .attribute_bounds
                        .sanitize(crate::mobs::finalize::Attr::MaxHealth, v)
                        as f32
                }
                "minecraft:movement_speed"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::MovementSpeed,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::MovementSpeed,
                        v,
                    )
                }
                "minecraft:movement_speed" => m.attrs.speed = v,
                "minecraft:attack_damage"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::AttackDamage,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::AttackDamage,
                        v,
                    )
                }
                "minecraft:attack_damage" => m.attrs.attack_damage = v as f32,
                "minecraft:attack_knockback" => m.attrs.attack_knockback = v,
                "minecraft:flying_speed"
                    if crate::mobs::finalize::base_value(
                        &m,
                        crate::mobs::finalize::Attr::FlyingSpeed,
                    ) != v =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::FlyingSpeed,
                        v,
                    )
                }
                // The attributes an effect's modifier changes (#1988).
                "minecraft:safe_fall_distance"
                | "minecraft:max_absorption"
                | "minecraft:waypoint_transmit_range" => {
                    use crate::mobs::finalize;
                    if let Some(a) = finalize::command_attr(id.trim_start_matches("minecraft:"))
                        && finalize::default_value(&m, a) != Some(v)
                    {
                        finalize::set_base(&mut m, a, v);
                    }
                }
                _ => {}
            }
        }
    }
    // `AttributeMap.apply`: the saved permanent modifiers of the attributes the
    // game models (the follow range bonus finalizeSpawn made among them), and
    // the other saved attributes for the live map.
    if let Some(Tag::List(_)) = tag.get("attributes") {
        use crate::mobs::finalize::Attr;
        saved_attributes::read_attribute_modifiers(&mut m, tag);
        // An attribute the type lacks is skipped (`AttributeMap.apply`).
        let kind = m.kind.clone();
        m.extra.live.load(tag.get("attributes"), |id| {
            kind.attributes.contains_key(id)
                && !Attr::ALL.iter().any(|a| *a != Attr::Armor && a.id() == id)
        });
        // A saved scale (an effect's modifier, `/attribute`) sizes the mob
        // as `onAttributeUpdated(SCALE)` does once the load dirtied it.
        if let Some(Tag::List(list)) = tag.get("attributes")
            && list
                .iter()
                .any(|entry| text(entry.get("id")).as_deref() == Some("minecraft:scale"))
        {
            crate::mobs::finalize::scale_changed(&mut m);
        }
    }
    m.health = loaded_health(&m, float(tag.get("Health")));
    // LivingEntity.readAdditionalSaveData: `internalSetAbsorptionAmount`
    // stores the value as read; only `setAbsorptionAmount` clamps.
    m.absorption = float(tag.get("AbsorptionAmount")).map_or(0.0, |h| h as f32);
    m.hurt_time = i32::from(number(tag.get("HurtTime")).unwrap_or(0) as i16);
    // What it holds (equipment.mainhand / offhand), when saved.
    if let Some(eq) = tag.get("equipment") {
        // The hands, armour, body and saddle: the one `equipment` codec.
        m.extra.equipment = sunrise_items::equipment::Equipment::read(eq);
        // Their enchantments and trims.
        // (`gear` slot order: hands, then head to feet.)
        use sunrise_items::equipment::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
        for (index, slot) in [MAINHAND, OFFHAND, HEAD, CHEST, LEGS, FEET]
            .into_iter()
            .enumerate()
        {
            if let Some(stack) = m.extra.slot_stack(slot) {
                let levels = stack.enchantments().to_vec();
                let trim = stack.data().and_then(|d| d.trim.clone());
                crate::mobs::gear::set(&mut m, index, levels);
                crate::mobs::gear::set_trim(&mut m, index, trim);
            }
        }
    } else {
        // Registration hands out default weapons (finalizeSpawn does in
        // vanilla); a loaded mob holds only what its file lists.
        m.extra.set_main_hand(None);
    }
    mob_death::read(&mut m, tag);
    m.ai.ai_paused = ai_pause::read(tag);
    m.impulse_grace_time =
        number(tag.get("current_impulse_context_reset_grace_time")).unwrap_or(0) as i32;
    // Mob.readAdditionalSaveData overrides what the constructor set (copper
    // golems and elder guardians start persistent).
    m.persistent = flag(tag, "PersistenceRequired");
    m.extra.death_loot_table = text(tag.get("DeathLootTable"))
        .and_then(|t| sunrise_game_data::identifier::normalize_id(&t));
    m.extra.death_table_seed = number(tag.get("DeathLootTableSeed")).unwrap_or(0);
    refs::read_last_hurt_by(&mut m, tag);
    // `NeutralMob.readPersistentAngerSaveData`: the anger end time and the
    // target come back with the mob.
    if persistent_anger(m.species()) {
        m.extra.anger_end = number(tag.get("anger_end_time")).unwrap_or(-1);
        // Anger at an entity other than the player is anger at a mob that may
        // not be here (`EntityReference`): it still is not `isAngryAtAllPlayers`.
        m.extra.angry_at = match uuid_from_tag(tag.get("angry_at")) {
            Some(uuid) if uuid == player_uuid() => Some(crate::mobs::Who::Player),
            Some(_) => Some(crate::mobs::Who::Mob(0)),
            None => None,
        };
    }
    // `Mannequin.readAdditionalSaveData`: the profile comes back with the skin, then
    // validated pose, `immovable`, `hide_description` and `description` (`updateDescription`).
    if m.species() == crate::mobs::Species::Mannequin {
        m.profile = tag.get("profile").and_then(profile_from_nbt);
        // Mannequin.POSE_CODEC accepts only these Avatar poses; an invalid
        // or absent value defaults to standing before the live dimensions read it.
        m.extra.avatar_pose = match text(tag.get("pose")).as_deref() {
            Some("crouching") => crate::pose::Pose::Crouching,
            Some("swimming") => crate::pose::Pose::Swimming,
            Some("fall_flying") => crate::pose::Pose::FallFlying,
            Some("sleeping") => crate::pose::Pose::Sleeping,
            _ => crate::pose::Pose::Standing,
        };
        // `HumanoidArm.CODEC`, right when absent or not an arm.
        m.extra.main_hand_left = text(tag.get("main_hand")).as_deref() == Some("left");
        // `LAYERS_CODEC`: every listed part hidden; an entry that is not a
        // part fails the whole list, which reads as `ALL_LAYERS` (none hidden).
        m.extra.hidden_model_parts = match tag.get("hidden_layers") {
            Some(Tag::List(parts)) => parts
                .iter()
                .try_fold(0u8, |mask, part| {
                    let name = text(Some(part))?;
                    let bit = MODEL_PARTS.iter().position(|p| *p == name)?;
                    Some(mask | 1 << bit)
                })
                .unwrap_or(0),
            _ => 0,
        };
        m.extra.immovable = flag(tag, "immovable");
        m.extra.description = if flag(tag, "hide_description") {
            crate::mobs::MannequinDescription::Hidden
        } else {
            crate::mobs::MannequinDescription::from_component(Some(
                &tag.get("description")
                    .cloned()
                    .or_else(|| crate::mobs::MannequinDescription::Default.component())
                    .unwrap_or(Tag::End),
            ))
        };
    }
    // AbstractPiglin.readAdditionalSaveData: getBooleanOr("CanPickUpLoot", true).
    let piglin = matches!(
        m.species(),
        crate::mobs::Species::Piglin | crate::mobs::Species::PiglinBrute
    );
    m.extra.pick_up_loot = tag
        .get("CanPickUpLoot")
        .and_then(int_value)
        .map_or(piglin, |value| value as i8 != 0);
    let decoded_name = tag.get("CustomName").and_then(decode_custom_name_text);
    m.custom_name = decoded_name.as_ref().map(crate::text::Text::plain);
    m.custom_name_text = decoded_name;
    m.extra.left_handed = flag(tag, "LeftHanded");
    if let Some(Tag::Compound(c)) = tag.get("drop_chances") {
        let mut chances = [crate::mobs::drops::DEFAULT_DROP_CHANCE; 8];
        for (chance, slot) in chances.iter_mut().zip(crate::mobs::drops::SLOTS) {
            if let Some(Tag::Float(v)) = c.get(&slot.into()) {
                *chance = f32::from_bits(*v);
            }
        }
        m.extra.drop_chances = Some(Box::new(chances));
    }
    if saves_age(m.species()) {
        m.extra.age = tag.get("Age").and_then(int_value).unwrap_or(0);
        m.extra.forced_age = tag.get("ForcedAge").and_then(int_value).unwrap_or(0);
        m.extra.age_locked = flag(tag, "AgeLocked");
    }
    if is_animal(&m.kind.id) {
        m.extra.in_love = tag.get("InLove").and_then(int_value).unwrap_or(0);
        // Animal.readAdditionalSaveData: the player who fed it into love.
        m.extra.love_cause = crate::save::uuid_from_tag(tag.get("LoveCause"));
    }
    m.name_visible = flag(tag, "CustomNameVisible");
    m.no_ai = flag(tag, "NoAI");
    m.no_gravity = flag(tag, "NoGravity");
    m.glowing = flag(tag, "Glowing");
    m.ticks_frozen = number(tag.get("TicksFrozen")).unwrap_or(0) as i32;
    // LivingEntity.readAdditionalSaveData: the saved `active_effects`, the last
    // entry of an effect winning.
    if let Some(Tag::List(effects)) = tag.get("active_effects") {
        for e in effects {
            let Some(def) = text(e.get("id")).and_then(|id| crate::brewing::effect(&id)) else {
                continue;
            };
            let effect = crate::brewing::namespaced(def.id);
            let duration = number(e.get("duration")).unwrap_or(0) as i32;
            let amplifier = effect_amplifier(e);
            // Details.CODEC: `show_particles` defaults to true, `show_icon` to `show_particles`.
            let particles = number(e.get("show_particles")) != Some(0);
            let mut stored = crate::mobs::StoredEffect::new(effect, duration, amplifier);
            stored.show_icon = number(e.get("show_icon")).map_or(particles, |v| v != 0);
            m.load_effect(stored);
            m.extra.effect_chains.retain(|e| e.id != effect);
            if let Some(hidden) = e.get("hidden_effect") {
                m.extra
                    .effect_chains
                    .push(hidden_effect_from_nbt(effect, hidden));
            }
            m.extra.hidden_effects.retain(|h| *h != effect);
            if !particles {
                m.extra.hidden_effects.push(effect);
            }
            // Details.CODEC: `ambient` defaults to false.
            m.extra.ambient_effects.retain(|h| *h != effect);
            if number(e.get("ambient")).is_some_and(|a| a != 0) {
                m.extra.ambient_effects.push(effect);
            }
        }
        // A loaded effect adds no modifier (`load_effect`): a loaded Health
        // Boost's maximum is the saved `attributes`' one, and the health
        // follows it as `setHealth` clamps.
        if m.extra
            .effects
            .iter()
            .any(|e| e.id == "minecraft:health_boost")
        {
            crate::mobs::finalize::health_boost_changed(&mut m);
        }
    }
    m.silent = flag(tag, "Silent");
    m.invulnerable_flag = flag(tag, "Invulnerable");
    // Entity.load: `getIntOr("invulnerable_time", 0)` — the entity-level
    // window (#916); the hurt cooldown never loads.
    m.invulnerable_time = match tag.get("invulnerable_time") {
        Some(Tag::Int(ticks)) => *ticks,
        _ => 0,
    };
    if m.species() == crate::mobs::Species::Bat {
        // Bat.readAdditionalSaveData: the raw BatFlags byte; bit 0 is resting.
        let flags = number(tag.get("BatFlags")).unwrap_or(0) as u8;
        m.extra.flag = flags & 1 != 0;
        m.extra.bat_flags = flags & !1;
    }
    if m.species() == crate::mobs::Species::Creeper {
        m.extra.charged = flag(tag, "powered");
        // Creeper.readAdditionalSaveData: `ignited` lights the fuse.
        m.extra.flag = flag(tag, "ignited");
        m.max_swell = number(tag.get("Fuse")).unwrap_or(30) as i16 as i32;
        // `getByteOr("ExplosionRadius", 3)`: any numeric tag, as a byte.
        m.extra.explosion_radius = number(tag.get("ExplosionRadius")).unwrap_or(3) as i8;
    }
    if m.species() == crate::mobs::Species::Ghast {
        crate::mobs::nether::from_nbt(&mut m, tag);
    }
    // InventoryCarrier.readInventoryFromTag, using the existing occupied-stack model.
    let carrier_slots = match m.species() {
        crate::mobs::Species::Pillager => 5,
        crate::mobs::Species::WanderingTrader => 8,
        _ => 0,
    };
    if carrier_slots != 0
        && let Some(Tag::List(items)) = tag.get("Inventory")
    {
        m.extra.inventory = items
            .iter()
            .filter_map(crate::save::item_from_nbt)
            .take(carrier_slots)
            .collect();
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Piglin
            | crate::mobs::Species::PiglinBrute
            | crate::mobs::Species::Hoglin
    ) {
        m.extra.flag2 = flag(tag, "IsImmuneToZombification");
        m.extra.counter2 = number(tag.get("TimeInOverworld")).unwrap_or(0) as i32;
        if m.species() == crate::mobs::Species::Piglin {
            crate::mobs::piglin::read_guarded_anger(&mut m, tag);
        }
        if m.species() == crate::mobs::Species::Hoglin {
            m.extra.cannot_be_hunted = flag(tag, "CannotBeHunted");
        }
        if m.species() == crate::mobs::Species::Piglin
            && let Some(Tag::List(items)) = tag.get("Inventory")
        {
            // InventoryCarrier.readInventoryFromTag -> SimpleContainer
            // .fromItemList (eight slots).
            m.extra.inventory = crate::mobs::villager::load_container(&m, items, 8);
        }
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Pig | crate::mobs::Species::Strider
    ) {
        // The old `Saddle` flag, when `equipment` held none.
        if flag(tag, "Saddle") && m.saddle_item().is_none() {
            m.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
        }
    }
    if crate::mobs::equine::is_equine(m.species()) {
        m.extra.tame = flag(tag, "Tame");
        // AbstractHorse.readAdditionalSaveData: the saved Int as is, camels included.
        m.extra.equine_temper = tag.get("Temper").and_then(int_value).unwrap_or(0);
        m.extra.bred = flag(tag, "Bred");
        // `setEating(getBooleanOr("EatingHaystack", false))`, camels included.
        m.extra.flag = flag(tag, "EatingHaystack");
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let v = number(tag.get("Variant")).unwrap_or(0) as i32;
        (m.extra.variant, m.extra.variant2) = crate::mobs::equine::horse_look(v);
        if crate::mobs::llama::is_llama(m.species()) {
            crate::mobs::llama::from_nbt(&mut m, tag);
        }
        m.extra.chested = flag(tag, "ChestedHorse");
        if m.extra.chested {
            m.extra.items = crate::save::decode_items(tag.get("Items"), 15);
        }
        if let Some(Tag::List(attrs)) = tag.get("attributes") {
            for a in attrs {
                let (Some(id), Some(v)) = (text(a.get("id")), float(a.get("base"))) else {
                    continue;
                };
                if id == "minecraft:jump_strength" {
                    m.extra.vector[0] = v;
                }
            }
        }
        if m.extra.vector[0] <= 0.0 {
            m.extra.vector[0] = if crate::mobs::equine::is_chested(m.species()) {
                0.5
            } else {
                0.7
            };
        }
    }
    if m.species() == crate::mobs::Species::Chicken
        && let Some(t) = number(tag.get("EggLayTime"))
    {
        m.extra.egg_time = t as i32;
    }
    match m.species() {
        crate::mobs::Species::Fox => {
            m.extra.variant = u8::from(text(tag.get("Type")).as_deref() == Some("snow"));
            // Fox.readAdditionalSaveData: clearTrusted, then addTrustedEntity for
            // each of "Trusted".
            crate::mobs::fox::clear_trusted(&mut m);
            if let Some(Tag::List(trusted)) = tag.get("Trusted") {
                for uuid in trusted
                    .iter()
                    .filter_map(|t| crate::save::uuid_from_tag(Some(t)))
                {
                    crate::mobs::fox::add_trusted_entity(&mut m, uuid);
                }
            }
            // Fox.readAdditionalSaveData: the sleeping, sitting and crouching flags.
            for (key, bit) in [
                ("Sleeping", crate::mobs::fox::SLEEPING),
                ("Sitting", crate::mobs::fox::SITTING),
                ("Crouching", crate::mobs::fox::CROUCHING),
            ] {
                if flag(tag, key) {
                    m.extra.ints[0] |= bit;
                }
            }
        }
        // MushroomCow.readAdditionalSaveData: a missing or unknown name is
        // Variant.DEFAULT, red; stew_effects decodes like the component's list.
        crate::mobs::Species::Mooshroom => {
            m.extra.variant = u8::from(text(tag.get("Type")).as_deref() == Some("brown"));
            if let Some(Tag::List(list)) = tag.get("stew_effects") {
                m.extra.stew_effects = list
                    .iter()
                    .map(|entry| {
                        Some((
                            Arc::from(text(entry.get("id"))?.as_str()),
                            number(entry.get("duration")).unwrap_or(160) as i32,
                        ))
                    })
                    .collect();
            }
        }
        crate::mobs::Species::Rabbit => {
            // Rabbit.Variant.byId: an unknown id is brown.
            m.extra.variant = match number(tag.get("RabbitType")).unwrap_or(0) {
                v @ (0..=5 | 99) => v as u8,
                _ => 0,
            };
        }
        crate::mobs::Species::Goat => crate::mobs::goat::read_nbt(&mut m, tag),
        _ => {}
    }
    // `Rabbit.readAdditionalSaveData`.
    if m.species() == crate::mobs::Species::Rabbit {
        m.extra.more_carrot_ticks = number(tag.get("MoreCarrotTicks")).unwrap_or(0) as i32;
    }
    // `Ocelot.readAdditionalSaveData`.
    if m.species() == crate::mobs::Species::Ocelot {
        m.extra.trusting = flag(tag, "Trusting");
    }
    if m.species() == crate::mobs::Species::Cat {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
        let legacy = |c: i64| u8::try_from(c as i8).ok().filter(|c| *c < 16).unwrap_or(0);
        m.extra.color = number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, legacy);
    }
    if m.species() == crate::mobs::Species::Panda {
        let gene = |key| text(tag.get(key)).map_or(0, |n| crate::mobs::panda::gene_from_name(&n));
        m.extra.variant = gene("MainGene");
        m.extra.variant2 = gene("HiddenGene");
    }
    if m.species() == crate::mobs::Species::Parrot {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
    }
    if m.species() == crate::mobs::Species::Wolf {
        let health = m.health;
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let tame = m.extra.owner.is_some();
        crate::mobs::tame::set_tame(&mut m, tame);
        m.health = health;
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
        let legacy = |c: i64| u8::try_from(c as i8).ok().filter(|c| *c < 16).unwrap_or(0);
        m.extra.color = number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, legacy);
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Villager | crate::mobs::Species::WanderingTrader
    ) {
        read_villager(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::ZombieVillager {
        read_villager(&mut m, tag);
        let xp = number(tag.get("Xp")).unwrap_or(0) as i32;
        if xp != 0 {
            m.extra.trade.get_or_insert_with(Default::default).xp = xp;
        }
        crate::mobs::zombie_villager::read_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Dolphin {
        // Dolphin.readAdditionalSaveData: a save that names neither field is an
        // unfed dolphin at full moistness.
        m.extra.flag = flag(tag, "GotFish");
        m.extra.counter = number(tag.get("Moistness")).unwrap_or(2400) as i32;
    }
    if m.species() == crate::mobs::Species::Axolotl {
        restore_axolotl_memories(&mut m, tag);
    }
    read_variant(&mut m, tag);
    m.fire_ticks = i32::from(number(tag.get("Fire")).unwrap_or(0) as i16);
    // Entity.load: `input.getIntOr("Air", getMaxAirSupply())`, 4800 for a dolphin.
    m.air = number(tag.get("Air")).map_or(m.max_air_supply(), |air| air as i32);
    if m.species() == crate::mobs::Species::Allay {
        if let Some(Tag::List(inv)) = tag.get("Inventory") {
            for stack in inv.iter().filter_map(item_from_nbt) {
                let max = m.kind.inventory_stack_limit(&stack);
                if let Some(st) = m.extra.allay.as_mut() {
                    st.add(stack, max);
                }
            }
        }
        m.extra.egg_time = number(tag.get("DuplicationCooldown")).unwrap_or(0) as i32;
        if let (Some(st), Some(listener)) = (m.extra.allay.as_mut(), tag.get("listener")) {
            st.vibration = crate::sculk::Data::from_nbt(listener);
        }
        let liked = tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get("minecraft:liked_player"));
        m.extra.tame = liked.is_some();
        m.extra.owner = liked
            .and_then(|l| l.get("value"))
            .and_then(|v| crate::save::uuid_from_tag(Some(v)));
        // `Brain.readAdditionalSaveData`: a memory saved with a time to live
        // (`Codec.LONG`, read from any integer tag). `Long.MAX_VALUE` is
        // `MemorySlot.NEVER_EXPIRE`: that memory never expires.
        if let Some(st) = m.extra.allay.as_mut() {
            st.liked_player_ttl = liked
                .and_then(|l| match l.get("ttl")? {
                    Tag::Byte(v) => Some(i64::from(*v)),
                    Tag::Short(v) => Some(i64::from(*v)),
                    Tag::Int(v) => Some(i64::from(*v)),
                    Tag::Long(v) => Some(*v),
                    _ => None,
                })
                .filter(|&ttl| ttl != i64::MAX);
        }
        let memory = |key: &str| {
            tag.get("Brain")
                .and_then(|b| b.get("memories"))
                .and_then(|b| b.get(key))
                .and_then(|l| l.get("value"))
        };
        m.extra.ints[2] =
            number(memory("minecraft:liked_noteblock_cooldown_ticks")).unwrap_or(0) as i32;
        let pickup = number(memory("minecraft:item_pickup_cooldown_ticks")).map(|t| t as i32);
        crate::mobs::allay::set_pickup_cooldown(&mut m, pickup);
        if let Some(st) = m.extra.allay.as_mut()
            && let Some(noteblock) = memory("minecraft:liked_noteblock")
            && let Some(Tag::IntArray(p)) = noteblock.get("pos")
            && p.len() == 3
        {
            st.noteblock = Some(crate::mobs::allay::GlobalPos {
                dimension: crate::mobs::allay::dimension_id(
                    &text(noteblock.get("dimension")).unwrap_or_default(),
                ),
                pos: [p[0], p[1], p[2]],
            });
        }
    }
    if m.species() == crate::mobs::Species::Frog
        && tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get("minecraft:is_pregnant"))
            .is_some()
    {
        crate::mobs::frog::set_pregnant(&mut m, true);
    }
    if crate::mobs::camel::is_camel(m.species()) {
        // The tick is a game time: `camel::finish_load` puts it on the mob's
        // clock once the world's time is known.
        let tick = match tag.get("LastPoseTick") {
            Some(Tag::Long(v)) => *v,
            _ => 0,
        };
        crate::mobs::camel::stand_up_instantly(&mut m);
        m.extra.pose_change_tick = tick;
        m.extra.camel_sitting = tick < 0;
        // Camels are always tame (`Camel.register` in Sunrise).
        m.extra.tame = true;
    }
    if crate::mobs::nautilus::is_nautilus(m.species()) {
        crate::mobs::nautilus::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::SulfurCube {
        crate::mobs::sulfur_cube::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Bee {
        crate::mobs::bee::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Creaking
        && let Some(Tag::IntArray(home)) = tag.get("home_pos")
        && home.len() == 3
    {
        crate::mobs::creaking::set_transient(&mut m, [home[0], home[1], home[2]]);
    }
    if m.species() == crate::mobs::Species::Turtle {
        // Turtle.readAdditionalSaveData: `home_pos` (else where it stands) and `has_egg`.
        m.extra.flag = flag(tag, "has_egg");
        m.extra.home = Some(match tag.get("home_pos") {
            Some(Tag::IntArray(home)) if home.len() == 3 => [home[0], home[1], home[2]],
            _ => [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ],
        });
    }
    if m.species() == crate::mobs::Species::Wither {
        m.extra.counter = number(tag.get("Invul")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Sniffer {
        crate::mobs::sniffer::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::CopperGolem {
        // CopperGolem.readAdditionalSaveData: -1 (unset) when missing.
        m.extra.vector[0] = match tag.get("next_weather_age") {
            Some(Tag::Long(v)) => *v as f64,
            _ => -1.0,
        };
        let weather = match tag.get("weather_state") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        m.extra.variant = crate::mobs::copper_golem::WEATHER
            .iter()
            .position(|w| *w == weather)
            .unwrap_or(0) as u8;
    }
    m.extra.leash_pending = crate::mobs::leash::pending_from_nbt(tag.get("leash"));
    if m.species() == crate::mobs::Species::Warden {
        // The brain starts afresh: a minute before it digs back down.
        crate::mobs::warden::finalize(&mut m, false, &mut Vec::new());
        if let Some(st) = m.extra.warden.as_mut() {
            if let Some(listener) = tag.get("listener") {
                st.vibration = crate::sculk::Data::from_nbt(listener);
            }
            // `Warden.readAdditionalSaveData`: the anger comes back as the list
            // of UUIDs it was saved as. Every suspect starts in the UUID map and
            // the anger ticks move the loaded ones into the live one, since a
            // suspect may be loaded long after the warden.
            if let Some(anger) = tag.get("anger") {
                st.anger =
                    crate::mobs::warden::AngerManagement::from_saved(warden_anger_from_nbt(anger));
            }
        }
        restore_warden_owned_memories(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Armadillo {
        let state = text(tag.get("state"));
        let scute_time = number(tag.get("scute_time")).map(|t| t as i32);
        crate::mobs::armadillo::load(
            &mut m,
            tag.get("Brain").is_some(),
            state.as_deref(),
            scute_time,
        );
    }
    if m.species() == crate::mobs::Species::Ravager {
        m.extra.counter = number(tag.get("AttackTick")).unwrap_or(0) as i32;
        m.extra.counter2 = number(tag.get("StunTick")).unwrap_or(0) as i32;
        m.extra.anim_ticks = number(tag.get("RoarTick")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Endermite {
        m.extra.counter = number(tag.get("Lifetime")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Vex {
        crate::mobs::vex::read_nbt(&mut m, tag);
    }
    // Bogged.readAdditionalSaveData: unsheared when absent.
    if m.species() == crate::mobs::Species::Bogged {
        m.extra.sheared = flag(tag, "sheared");
    }
    // Sheep.readAdditionalSaveData: white and unsheared when absent.
    if m.species() == crate::mobs::Species::Sheep {
        m.extra.color = crate::mobs::farm::saved_color(tag.get("Color"));
        m.extra.sheared = flag(tag, "Sheared");
    }
    // GlowSquid.readAdditionalSaveData: not dimmed when absent.
    if m.species() == crate::mobs::Species::GlowSquid {
        crate::mobs::aquatic::set_dark_ticks(
            &mut m,
            number(tag.get("DarkTicksRemaining")).unwrap_or(0) as i32,
        );
    }
    // IronGolem.readAdditionalSaveData: not player created when absent.
    if m.species() == crate::mobs::Species::IronGolem {
        m.extra.player_created = flag(tag, "PlayerCreated");
    }
    // SnowGolem.readAdditionalSaveData: getBooleanOr("Pumpkin", true).
    if m.species() == crate::mobs::Species::SnowGolem {
        m.extra.sheared = !tag
            .get("Pumpkin")
            .and_then(int_value)
            .is_none_or(|value| value as i8 != 0);
    }
    // Pufferfish.readAdditionalSaveData: `getIntOr("PuffState", 0)`, at most 2.
    if m.species() == crate::mobs::Species::Pufferfish {
        m.extra.size = number(tag.get("PuffState")).unwrap_or(0).min(2) as i32;
    }
    // Enderman.readAdditionalSaveData: the block it carried; an air state
    // reads as no block at all.
    if m.species() == crate::mobs::Species::Enderman
        && let Some(registry) = registry
    {
        m.extra.carried = tag
            .get("carriedBlockState")
            .and_then(|t| state_from_nbt(registry, t))
            .filter(|&state| !is_air(registry, state));
    }
    // HappyGhast.readAdditionalSaveData: the still timeout, 0 when absent (a fresh
    // ghast, or one saved by a version that did not write it).
    if m.species() == crate::mobs::Species::HappyGhast {
        m.extra.counter = number(tag.get("still_timeout")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Shulker {
        // `Shulker.setDeltaMovement` ignores the loaded `Motion`.
        m.velocity = Vec3::ZERO;
        m.extra.variant3 = (number(tag.get("AttachFace")).unwrap_or(0) as i8 % 6).unsigned_abs();
        m.extra.counter = number(tag.get("Peek")).map_or(0, |v| v as i8 as i32);
        m.extra.color = number(tag.get("Color")).map_or(16, |v| v as i8 as u8);
        m.anim.v[0] = m.extra.counter as f32 * 0.01;
        m.anim.vo[0] = m.anim.v[0];
        // Shulker.setPos: square on its block.
        m.position = sunrise_core::geometry::Vec3::new(
            m.position.x.floor() + 0.5,
            (m.position.y + 0.5).floor(),
            m.position.z.floor() + 0.5,
        );
        m.previous = m.position;
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    ) {
        m.extra.flag2 = flag(tag, "wasOnGround");
    }
    if m.species() == crate::mobs::Species::Phantom {
        // Phantom.readAdditionalSaveData: setPhantomSize(size), and the circling
        // anchor, absent or unreadable when the save has no anchor at all
        // (`input.read("anchor_pos", BlockPos.CODEC).orElse(null)`).
        crate::mobs::phantom::set_size(&mut m, number(tag.get("size")).unwrap_or(0) as i32);
        m.extra.home = home_block_pos(tag.get("anchor_pos"));
    }
    // Zombie, Piglin and Zoglin.readAdditionalSaveData: IsBaby (the baby
    // zombies of trial spawners).
    if !crate::mobs::finalize::ageable(m.species()) {
        crate::mobs::finalize::set_baby(&mut m, flag(tag, "IsBaby"));
    }
    // Zombie.readAdditionalSaveData: setCanBreakDoors.
    if crate::mobs::doors::is_zombie(m.species()) {
        crate::mobs::doors::set_can_break_doors(&mut m, flag(tag, "CanBreakDoors"));
    }
    // Zombie and Skeleton.readAdditionalSaveData: `ConversionTracker` reads its
    // affliction time (0 when absent) and starts a conversion from a saved time
    // other than -1, -1 stopping one (`setConverting(false)`).
    let tracker = match m.species() {
        crate::mobs::Species::Zombie | crate::mobs::Species::Husk => {
            Some(("InWaterTime", "DrownedConversionTime"))
        }
        crate::mobs::Species::Skeleton => Some(("FreezingTime", "StrayConversionTime")),
        _ => None,
    };
    if let Some((affliction, conversion)) = tracker {
        m.extra.counter = number(tag.get(affliction)).unwrap_or(0) as i32;
        let time = number(tag.get(conversion)).unwrap_or(-1) as i32;
        // Any other time starts a conversion (`startConversion`), a negative
        // one spent on the next tick.
        m.extra.counter2 = if time == -1 { 0 } else { time.max(0) + 1 };
    }
    // SpellcasterIllager.readAdditionalSaveData: this is the same timer used
    // by casting goals and the existing customServerAiStep countdown.
    if matches!(
        m.species(),
        crate::mobs::Species::Evoker | crate::mobs::Species::Illusioner
    ) {
        m.extra.counter = number(tag.get("SpellTicks")).unwrap_or(0) as i32;
    }
    // Vindicator.readAdditionalSaveData: `Johnny`, false when absent. A name
    // that is Johnny latches the flag again on the next AI step.
    if m.species() == crate::mobs::Species::Vindicator {
        m.extra.johnny = flag(tag, "Johnny");
    }
    crate::mobs::raider::read_nbt(&mut m, tag);
    crate::mobs::riders::read_nbt(&mut m, tag);
    // A mannequin is an `Avatar`, not a `Mob`: it reads none of
    // `Mob.readAdditionalSaveData`'s flags, after the common reads above.
    if m.species() == crate::mobs::Species::Mannequin {
        m.persistent = false;
        m.extra.pick_up_loot = false;
        m.extra.left_handed = false;
        m.no_ai = false;
    }
    // setHealth clamps a negative health to 0; a dead mob keeps its body (tickDeath).
    if m.health < 0.0 {
        m.health = 0.0;
    }
    m
}
