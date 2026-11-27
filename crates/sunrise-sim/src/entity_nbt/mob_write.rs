// Included in entity_nbt.rs to retain its private scope.
/// The mob's entity compound. `registry` is what writes a field holding a block
/// state (an enderman's `carriedBlockState`): without one, a caller that has no
/// resources (unit tests, structure templates) keeps whatever the loaded
/// compound had for it.
///
/// `uuid_of` is how a field that names another entity by UUID (a mob's
/// `angry_at`) is written: it answers the UUID of a mob by its id, from the game
/// layer's map, and `None` for one the level no longer has. A caller with no
/// such map passes `|_| None` and writes no reference.
pub fn mob_to_nbt(
    m: &Mob,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    registry: Option<&BlockRegistry>,
    uuid_of: impl FnMut(u32) -> Option<[i32; 4]>,
    items: &crate::items::Items,
) -> Tag {
    let mut retained = match raw {
        Some(tag @ Tag::Compound(_)) => tag.clone(),
        _ => Tag::Compound(Compound::new()),
    };
    ai_pause::write_ai_paused(m.ai.ai_paused, &mut retained);
    let mut out = EntityTagWriter::from_saved(retained);
    write_mob(
        m,
        uuid,
        &mut out,
        registry,
        uuid_of,
        items,
        MobSave::default(),
    );
    out.finish()
}

/// Runtime state shared by chunk saving and inspecting a live mob.
pub struct MobSave<'a> {
    pub include_id: bool,
    pub game_time: i64,
    pub variants: Option<&'a crate::mobs::variants::VariantTables>,
}

impl Default for MobSave<'_> {
    fn default() -> Self {
        Self {
            include_id: true,
            game_time: 0,
            variants: None,
        }
    }
}

/// The world supplies its live leash and hurt-by projection before writing.
pub fn write_mob(
    m: &Mob,
    uuid: [i32; 4],
    map: &mut EntityTagWriter,
    registry: Option<&BlockRegistry>,
    mut uuid_of: impl FnMut(u32) -> Option<[i32; 4]>,
    items: &crate::items::Items,
    mode: MobSave<'_>,
) {
    let MobSave {
        include_id,
        game_time,
        variants,
    } = mode;
    use crate::mobs::Species as S;
    let base = Base {
        id: &m.kind.id,
        uuid,
        position: m.position,
        motion: m.velocity,
        rotation: [m.yaw, m.head_pitch],
        on_ground: m.ground.on_ground,
        fall_distance: m.fall_distance,
    };
    // `Entity.saveWithoutId` consumes these live values in its own order.
    map.saved.insert(
        "Fire".into(),
        Tag::Short(m.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16),
    );
    map.saved.insert("Air".into(), Tag::Short(m.air as i16));
    map.saved.insert(
        "Invulnerable".into(),
        Tag::Byte(i8::from(m.invulnerable_flag)),
    );
    for (key, on) in [
        ("CustomNameVisible", m.name_visible),
        ("Silent", m.silent),
        ("NoGravity", m.no_gravity),
        ("Glowing", m.glowing),
    ] {
        if on {
            map.saved.insert(key.into(), Tag::Byte(1));
        } else {
            map.saved.remove(&key.into());
        }
    }
    if m.custom_name_changed(map.saved.get(&"CustomName".into()))
        && let Some(name) = m.custom_name_tag()
    {
        map.saved.insert("CustomName".into(), name);
    }
    for (key, time) in [
        ("invulnerable_time", m.invulnerable_time),
        ("TicksFrozen", m.ticks_frozen),
    ] {
        if time > 0 {
            map.saved.insert(key.into(), Tag::Int(time));
        } else {
            map.saved.remove(&key.into());
        }
    }
    base.write(map, include_id);

    // LivingEntity.addAdditionalSaveData.
    mob_death::write(m, map);
    map.insert(
        "AbsorptionAmount".into(),
        Tag::Float(m.absorption.to_bits()),
    );
    map.insert(
        "current_impulse_context_reset_grace_time".into(),
        Tag::Int(m.impulse_grace_time),
    );
    map.retain("current_explosion_impact_pos");
    if matches!(
        m.kind.class_id(),
        "minecraft:horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:skeleton_horse"
            | "minecraft:zombie_horse"
            | "minecraft:llama"
            | "minecraft:trader_llama"
    ) {
        // AttributeMap.pack writes the instances that exist: the ones the
        // entity was loaded with. When the tag had no attributes list,
        // instances are not synthesized (SummonCommand without finalizeSpawn
        // saves attributes: []).
        if let Some(Tag::List(list)) = map.get(&"attributes".into()) {
            let modelled = [
                ("minecraft:max_health", f64::from(m.attrs.max_health)),
                ("minecraft:movement_speed", m.attrs.speed),
                ("minecraft:jump_strength", m.extra.vector[0]),
            ];
            let attributes = list
                .iter()
                .map(|a| {
                    let id = text(a.get("id"));
                    let value = modelled.iter().find(|(m, _)| Some(*m) == id.as_deref());
                    match (a, value) {
                        (Tag::Compound(c), Some((_, v))) => {
                            let mut c = c.clone();
                            c.insert("base".into(), Tag::Double(v.to_bits()));
                            Tag::Compound(c)
                        }
                        _ => a.clone(),
                    }
                })
                .collect();
            map.insert("attributes".into(), Tag::List(attributes));
        }
    }
    if m.extra.attributes.is_some() {
        put_attributes(map, m);
    }
    put_sulfur_attributes(map, m);
    // The instances their own construction and load make (finalize::made_at_load),
    // which AttributeMap.pack saves although the loaded tag had none.
    use crate::mobs::finalize::Attr;
    let made = crate::mobs::finalize::made_at_load(m);
    // AttributeMap.pack writes the list even when no instance exists.
    map.entry("attributes".into())
        .or_insert_with(|| Tag::List(Vec::new()));
    for attr in made {
        let base = crate::mobs::finalize::made_base(m, attr);
        let mut list = match map.get(&"attributes".into()) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        let attr_id = format!("minecraft:{}", attr.name());
        if let Some(existing) = list
            .iter_mut()
            .find(|t| text(t.get("id")).as_deref() == Some(attr_id.as_str()))
        {
            // The model owns a wolf's health base.
            if (m.species(), attr) == (S::Wolf, Attr::MaxHealth)
                && let Tag::Compound(c) = existing
            {
                c.insert("base".into(), Tag::Double(base.to_bits()));
            }
        } else {
            list.push(Tag::compound([
                ("id", string(&attr_id)),
                ("base", Tag::Double(base.to_bits())),
            ]));
        }
        map.insert("attributes".into(), Tag::List(list));
    }
    put_live_attributes(&mut map.fields, &m.extra.live);
    // LivingEntity.addAdditionalSaveData: `active_effects` only when there are
    // any (MobEffectInstance.CODEC), including their modeled weaker instances.
    if m.extra.effects.is_empty() {
        map.remove(&sunrise_core::nbt::JavaString::from("active_effects"));
    } else {
        let effects = m.extra.effects.iter().map(|effect| {
            let show_icon = effect.show_icon;
            let (effect, duration, amplifier) = (effect.id, effect.duration, effect.amplifier);
            let shown = !m.extra.hidden_effects.contains(&effect);
            let ambient = m.extra.ambient_effects.contains(&effect);
            let mut e = vec![("id", string(crate::brewing::namespaced(effect)))];
            // The codec leaves out an amplifier of 0 (its default).
            if amplifier != 0 {
                e.push(("amplifier", Tag::Byte(amplifier.clamp(0, 255) as u8 as i8)));
            }
            e.push(("duration", Tag::Int(duration)));
            if ambient {
                e.push(("ambient", Tag::Byte(1)));
            }
            if !shown {
                e.push(("show_particles", Tag::Byte(0)));
            }
            e.push(("show_icon", Tag::Byte(i8::from(show_icon))));
            if let Some(hidden) = m.extra.effect_chains.iter().find(|e| e.id == effect) {
                e.push(("hidden_effect", hidden_effect_nbt(hidden)));
            }
            Tag::compound(e)
        });
        map.insert("active_effects".into(), Tag::List(effects.collect()));
    }
    map.insert(
        "FallFlying".into(),
        Tag::Byte(i8::from(m.extra.fall_flying)),
    );
    if m.species() == S::Villager {
        if let Some(pos) = crate::mobs::villager::sleeping_pos(m) {
            map.insert("sleeping_pos".into(), Tag::IntArray(pos.to_vec()));
        } else {
            map.remove(&"sleeping_pos".into());
        }
    } else {
        map.retain("sleeping_pos");
    }
    map.retain_or("Brain", || {
        Tag::compound([("memories", Tag::Compound(Compound::new()))])
    });
    map.retain("last_hurt_by_player");
    map.retain("last_hurt_by_player_memory_time");
    map.retain("last_hurt_by_mob");
    map.retain("ticks_since_last_hurt_by_mob");
    // LivingEntity equipment: what the mob holds and wears now.
    {
        let mut equipment = m.extra.equipment.clone();
        let head = sunrise_items::equipment::HEAD;
        // Held and worn items are saved with their enchantments and trim.
        for slot in sunrise_items::equipment::MAINHAND..=head {
            equipment[slot] = crate::mobs::finalize::item_stack(m, slot);
        }
        // The captain's ominous banner keeps its patterns.
        if equipment[head].as_ref().is_some_and(|banner| {
            &*banner.item == crate::mobs::raider::BANNER
                && banner.extra.is_none()
                && crate::mobs::raider::is_raider(m.species())
        }) {
            equipment[head] = Some(crate::mobs::raider::ominous_banner());
        }
        if let Some(equipment) = equipment.write(items) {
            map.insert("equipment".into(), equipment);
        } else {
            map.remove(&"equipment".into());
        }
    }
    map.retain("locator_bar_icon");
    if m.species() != S::Mannequin {
        // Mob.addAdditionalSaveData.
        map.insert(
            "CanPickUpLoot".into(),
            Tag::Byte(i8::from(m.can_pick_up_loot())),
        );
        map.insert(
            "PersistenceRequired".into(),
            Tag::Byte(i8::from(m.persistent)),
        );
        // Mob.addAdditionalSaveData: the chances that are not the default.
        if let Some(chances) = &m.extra.drop_chances {
            let set: Vec<(&str, Tag)> = crate::mobs::drops::SLOTS
                .iter()
                .zip(chances.iter())
                .filter(|(_, c)| **c != crate::mobs::drops::DEFAULT_DROP_CHANCE)
                .map(|(slot, c)| (*slot, Tag::Float(c.to_bits())))
                .collect();
            if set.is_empty() {
                map.remove(&"drop_chances".into());
            } else {
                map.insert("drop_chances".into(), Tag::compound(set));
            }
        }
        map.retain("leash");
        // Mob.addAdditionalSaveData: hasHome means radius != -1, including -2.
        // Species writers below retain their own home_pos override.
        if let Some((pos, radius)) = m.ai.home.filter(|(_, radius)| *radius != -1) {
            map.insert("home_radius".into(), Tag::Int(radius));
            map.insert("home_pos".into(), Tag::IntArray(pos.to_vec()));
        } else {
            map.remove(&"home_radius".into());
            map.remove(&"home_pos".into());
        }
        map.insert(
            "LeftHanded".into(),
            Tag::Byte(i8::from(m.extra.left_handed)),
        );
        // Mob.addAdditionalSaveData: the death loot table, and its seed when set.
        if let Some(table) = &m.extra.death_loot_table {
            map.insert(("DeathLootTable").into(), string(table));
        }
        if m.extra.death_table_seed != 0 {
            map.insert(
                ("DeathLootTableSeed").into(),
                Tag::Long(m.extra.death_table_seed),
            );
        }
        if m.no_ai {
            map.insert("NoAI".into(), Tag::Byte(1));
        } else {
            map.remove(&"NoAI".into());
        }
    } else {
        // Mannequin is an Avatar, not a Mob; discard loaded Mob-only data.
        for key in MOB_ONLY_KEYS {
            map.remove(&key.into());
        }
    }
    // Keys of older formats that structure templates still carry.
    for key in [
        "HandItems",
        "ArmorItems",
        "HandDropChances",
        "ArmorDropChances",
        "Dimension",
        "HurtByTimestamp",
        "Leashed",
    ] {
        map.remove(&sunrise_core::nbt::JavaString::from(key));
    }
    // `Mannequin.addAdditionalSaveData`: the profile whose skin it wears, whether it
    // is `immovable`, and its description (the default is not written; none is `hide_description`).
    if m.species() == crate::mobs::Species::Mannequin {
        // The empty static profile (`DEFAULT_PROFILE`) when it has none.
        let profile = m
            .profile
            .as_ref()
            .map_or_else(|| Tag::Compound(Compound::new()), profile_nbt);
        map.insert(("profile").into(), profile);
        // `LAYERS_CODEC`: the hidden layers, in `PlayerModelPart` order.
        let hidden = MODEL_PARTS
            .iter()
            .enumerate()
            .filter(|(bit, _)| m.extra.hidden_model_parts & (1 << bit) != 0)
            .map(|(_, part)| string(part))
            .collect();
        map.insert(("hidden_layers").into(), Tag::List(hidden));
        let hand = if m.extra.main_hand_left {
            "left"
        } else {
            "right"
        };
        map.insert(("main_hand").into(), string(hand));
        let pose = match m.extra.avatar_pose {
            crate::pose::Pose::Crouching => "crouching",
            crate::pose::Pose::Swimming => "swimming",
            crate::pose::Pose::FallFlying => "fall_flying",
            crate::pose::Pose::Sleeping => "sleeping",
            _ => "standing",
        };
        map.insert(("pose").into(), string(pose));
        map.insert(("immovable").into(), Tag::Byte(i8::from(m.extra.immovable)));
        match &m.extra.description {
            crate::mobs::MannequinDescription::Default => {}
            crate::mobs::MannequinDescription::Hidden => {
                map.insert(("hide_description").into(), Tag::Byte(1));
            }
            crate::mobs::MannequinDescription::Custom(component) => {
                map.insert(("description").into(), component.clone());
            }
        }
    }
    // AbstractFish.addAdditionalSaveData always writes the flag.
    // Tadpole.fromBucket is always true, and its setter ignores loaded input.
    let from_bucket = match m.species() {
        S::Tadpole => Some(true),
        S::Cod | S::Salmon | S::Pufferfish | S::TropicalFish => Some(m.extra.from_bucket),
        _ => None,
    };
    if let Some(value) = from_bucket {
        map.insert(("FromBucket").into(), Tag::Byte(i8::from(value)));
    }
    // What the mob's class saves: a content mod's living type saves as its
    // template does (the `id` written above stays its own).
    let id = m.kind.class_id();
    // AgeableMob.addAdditionalSaveData, only if canBeABaby (not a frog or a
    // parrot); a tadpole saves its own age.
    let species = m.species();
    if saves_age(species) {
        map.insert("Age".into(), Tag::Int(m.extra.age));
        if species != S::Tadpole {
            map.insert("ForcedAge".into(), Tag::Int(m.extra.forced_age));
        }
        map.insert("AgeLocked".into(), Tag::Byte(i8::from(m.extra.age_locked)));
    } else if crate::mobs::finalize::ageable(species) {
        // AgeableMob.addAdditionalSaveData writes nothing when canBeABaby is
        // false — the frog, the parrot, the zombie nautilus, the camel husk,
        // the slime and the magma cube: drop the ignored values a raw input
        // compound carried rather than writing them back out.
        for key in ["Age", "ForcedAge", "AgeLocked"] {
            map.remove(&key.into());
        }
    }
    // Animal.addAdditionalSaveData: the love ticks left, whatever the save had.
    if is_animal(id) {
        map.insert("InLove".into(), Tag::Int(m.extra.in_love));
        // Animal.addAdditionalSaveData: the player who fed it.
        if let Some(cause) = m.extra.love_cause {
            map.insert("LoveCause".into(), Tag::IntArray(cause.to_vec()));
        } else {
            map.remove(&"LoveCause".into());
        }
    }
    // Zombie.addAdditionalSaveData precedes every Zombie subclass.
    if crate::mobs::doors::is_zombie(species) {
        map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
        map.insert(
            "CanBreakDoors".into(),
            Tag::Byte(i8::from(m.extra.can_break_doors)),
        );
        // ConversionTracker writes the conversion time before the affliction time.
        if matches!(species, S::Zombie | S::Husk) {
            map.insert("DrownedConversionTime".into(), Tag::Int(conversion_time(m)));
        } else {
            map.entry("DrownedConversionTime".into())
                .or_insert(Tag::Int(-1));
        }
        if !m.fluids.eye_in_water {
            map.insert("InWaterTime".into(), Tag::Int(-1));
        } else if matches!(species, S::Zombie | S::Husk) {
            map.insert("InWaterTime".into(), Tag::Int(m.extra.counter));
        } else {
            // Not counting for this kind: the drowning tracker writes its time as loaded, 0 by default.
            map.retain_or("InWaterTime", || Tag::Int(0));
        }
    }
    // PatrollingMonster and Raider write before their subclasses.
    crate::mobs::raider::write_nbt(m, map);
    if matches!(species, S::Villager | S::WanderingTrader) {
        // AbstractVillager writes offers before its inventory.
        put_offers(map, m, items);
    }
    // InventoryCarrier.writeInventoryToTag: the occupied stacks, even when empty.
    if matches!(id, "minecraft:pillager" | "minecraft:wandering_trader") {
        map.insert(
            "Inventory".into(),
            Tag::List(
                m.extra
                    .inventory
                    .iter()
                    .map(|stack| crate::save::item_to_nbt(stack, items))
                    .collect(),
            ),
        );
    }
    // TamableAnimal precedes Cat, Wolf and Parrot.
    if matches!(species, S::Cat | S::Wolf | S::Parrot) {
        if let Some(owner) = m.extra.owner {
            map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
        } else {
            map.remove(&"Owner".into());
        }
        map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
    }
    if species == S::Wolf {
        map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
    }
    if species == S::Chicken {
        crate::mobs::riders::write_nbt(m, map);
        map.insert("EggLayTime".into(), Tag::Int(m.extra.egg_time));
    }
    // `VariantUtils.writeVariant`: only the species that store the name. A
    // mooshroom saves `Type` (below), a fox `Type`, a horse and a llama a
    // packed `Variant` int, so those report the component but save no name.
    // ZombieNautilus writes its variant after the TamableAnimal fields below.
    match species {
        S::Wolf => {
            if let Some(variants) = variants
                && let Some(r) = m.wolf_variant(&variants.wolf)
                && let Some(id) = variants.wolf.id(r)
            {
                map.insert("variant".into(), string(id));
            }
        }
        S::Cat => {
            if let Some(variants) = variants
                && let Some(r) = m.cat_variant(&variants.cat)
                && let Some(id) = variants.cat.id(r)
            {
                map.insert("variant".into(), string(id));
            }
        }
        S::Pig => {
            if let Some(variants) = variants
                && let Some(r) = m.pig_variant(&variants.pig)
                && let Some(id) = variants.pig.id(r)
            {
                map.insert("variant".into(), string(id));
            }
        }
        S::Cow => {
            if let Some(variants) = variants
                && let Some(r) = m.cow_variant(&variants.cow)
                && let Some(id) = variants.cow.id(r)
            {
                map.insert("variant".into(), string(id));
            }
        }
        S::Chicken => {
            if let Some(variants) = variants
                && let Some(r) = m.chicken_variant(&variants.chicken)
                && let Some(id) = variants.chicken.id(r)
            {
                map.insert("variant".into(), string(id));
            }
        }
        S::Frog => {
            if let Some(variants) = variants
                && let Some(r) = m.frog_variant(&variants.frog)
                && let Some(id) = variants.frog.id(r)
            {
                map.insert("variant".into(), string(id));
            }
        }
        _ => {}
    }
    match species {
        S::Wolf => {
            write_persistent_anger(m, map, &mut uuid_of);
            if let Some(variants) = variants
                && let Some(r) = m.wolf_sound_variant(&variants.wolf_sound)
                && let Some(id) = variants.wolf_sound.id(r)
            {
                map.insert("sound_variant".into(), string(id));
            }
        }
        S::Cat => {
            if let Some(variants) = variants
                && let Some(r) = m.cat_sound_variant(&variants.cat_sound)
                && let Some(id) = variants.cat_sound.id(r)
            {
                map.insert("sound_variant".into(), string(id));
            }
        }
        S::Pig => {
            if let Some(variants) = variants
                && let Some(r) = m.pig_sound_variant(&variants.pig_sound)
                && let Some(id) = variants.pig_sound.id(r)
            {
                map.insert("sound_variant".into(), string(id));
            }
        }
        S::Cow => {
            if let Some(variants) = variants
                && let Some(r) = m.cow_sound_variant(&variants.cow_sound)
                && let Some(id) = variants.cow_sound.id(r)
            {
                map.insert("sound_variant".into(), string(id));
            }
        }
        S::Chicken => {
            if let Some(variants) = variants
                && let Some(r) = m.chicken_sound_variant(&variants.chicken_sound)
                && let Some(id) = variants.chicken_sound.id(r)
            {
                map.insert("sound_variant".into(), string(id));
            }
        }
        _ => {}
    }
    match id {
        // Fox.addAdditionalSaveData.
        "minecraft:fox" => {
            // The trusted UUIDs (`Fox.TRUSTED_LIST_CODEC`: a list of int arrays).
            map.insert(
                "Trusted".into(),
                Tag::List(
                    crate::mobs::fox::trusted_entities(m)
                        .map(|uuid| Tag::IntArray(uuid.to_vec()))
                        .collect(),
                ),
            );
            map.insert(
                "Sleeping".into(),
                Tag::Byte(i8::from(crate::mobs::fox::flag(
                    m,
                    crate::mobs::fox::SLEEPING,
                ))),
            );
            let kind = if m.extra.variant == 1 { "snow" } else { "red" };
            map.insert("Type".into(), string(kind));
            for (key, bit) in [
                ("Sitting", crate::mobs::fox::SITTING),
                ("Crouching", crate::mobs::fox::CROUCHING),
            ] {
                map.insert(
                    key.into(),
                    Tag::Byte(i8::from(crate::mobs::fox::flag(m, bit))),
                );
            }
        }
        // MushroomCow.addAdditionalSaveData: the Variant codec's name and the
        // stew effects, written only while it holds any (storeNullable).
        "minecraft:mooshroom" => {
            let kind = if m.extra.variant == 1 { "brown" } else { "red" };
            map.insert("Type".into(), string(kind));
            match &m.extra.stew_effects {
                Some(effects) => {
                    let list = effects.iter().map(|(id, duration)| {
                        let mut entry = vec![("id", string(id))];
                        // SuspiciousStewEffects.Entry leaves out the default.
                        if *duration != 160 {
                            entry.push(("duration", Tag::Int(*duration)));
                        }
                        Tag::compound(entry)
                    });
                    map.insert("stew_effects".into(), Tag::List(list.collect()));
                }
                None => {
                    map.remove(&sunrise_core::nbt::JavaString::from("stew_effects"));
                }
            }
        }
        // Rabbit.addAdditionalSaveData (the legacy id; 99 is the killer bunny).
        "minecraft:rabbit" => {
            map.insert("RabbitType".into(), Tag::Int(i32::from(m.extra.variant)));
            map.insert(
                "MoreCarrotTicks".into(),
                Tag::Int(m.extra.more_carrot_ticks),
            );
        }
        "minecraft:goat" => crate::mobs::goat::write_nbt(m, map),
        "minecraft:creeper" => {
            map.insert("powered".into(), Tag::Byte(i8::from(m.extra.charged)));
            map.insert("Fuse".into(), Tag::Short(m.max_swell as i16));
            map.insert(
                "ExplosionRadius".into(),
                Tag::Byte(m.extra.explosion_radius),
            );
            map.insert("ignited".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        // `Ocelot.addAdditionalSaveData`.
        "minecraft:ocelot" => {
            map.insert("Trusting".into(), Tag::Byte(i8::from(m.extra.trusting)));
        }
        "minecraft:cat" => {
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
        }
        "minecraft:panda" => {
            use crate::mobs::panda::GENES;
            for (key, gene) in [
                ("MainGene", m.extra.variant),
                ("HiddenGene", m.extra.variant2),
            ] {
                map.insert(
                    key.into(),
                    string(GENES[usize::from(gene).min(GENES.len() - 1)]),
                );
            }
        }
        "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:hoglin" => {
            map.insert(
                "IsImmuneToZombification".into(),
                Tag::Byte(i8::from(m.extra.flag2)),
            );
            map.insert("TimeInOverworld".into(), Tag::Int(m.extra.counter2));
            if id == "minecraft:hoglin" {
                map.insert(
                    "CannotBeHunted".into(),
                    Tag::Byte(m.extra.cannot_be_hunted as i8),
                );
            }
            if species == S::PiglinBrute {
                map.remove(&"CannotHunt".into());
            } else if species == S::Piglin {
                map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
                map.insert(
                    "CannotHunt".into(),
                    Tag::Byte(m.extra.piglin.cannot_hunt as i8),
                );
                crate::mobs::piglin::write_guarded_anger(m, map);
                // InventoryCarrier.writeInventoryToTag.
                map.insert(
                    "Inventory".into(),
                    Tag::List(
                        m.extra
                            .inventory
                            .iter()
                            .map(|stack| crate::save::item_to_nbt(stack, items))
                            .collect(),
                    ),
                );
            }
        }
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:llama"
        | "minecraft:trader_llama"
        | "minecraft:camel"
        | "minecraft:camel_husk" => {
            // AbstractHorse records these before any subclass fields.
            let camel = matches!(species, S::Camel | S::CamelHusk);
            // AbstractHorse.addAdditionalSaveData: `putBoolean("EatingHaystack", isEating())`.
            map.insert("EatingHaystack".into(), Tag::Byte(i8::from(m.extra.flag)));
            map.insert("Bred".into(), Tag::Byte(i8::from(m.extra.bred)));
            map.insert("Temper".into(), Tag::Int(m.extra.equine_temper));
            map.insert("Tame".into(), Tag::Byte(i8::from(m.extra.tame)));
            if !camel && let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.retain("Owner");
            }
            if camel {
                // LastPoseTick is negative while sitting.
                map.insert(
                    "LastPoseTick".into(),
                    Tag::Long(crate::mobs::camel::synced_pose_tick(m, game_time)),
                );
            }
            if id == "minecraft:horse" {
                let loaded = map.get(&"Variant".into()).and_then(int_value);
                let look = (m.extra.variant, m.extra.variant2);
                let v = crate::mobs::equine::saved_variant(loaded, look.0, look.1);
                map.insert("Variant".into(), Tag::Int(v));
            }
            let llama = crate::mobs::llama::is_llama(m.species());
            // Only llamas, donkeys and mules carry chests.
            if llama || matches!(id, "minecraft:donkey" | "minecraft:mule") {
                map.insert("ChestedHorse".into(), Tag::Byte(i8::from(m.extra.chested)));
            }
            if !camel && m.extra.chested && !m.extra.items.is_empty() {
                map.insert(
                    "Items".into(),
                    crate::save::encode_items(&m.extra.items, items),
                );
            } else if !camel {
                map.remove(&"Items".into());
            }
            // Llama and TraderLlama follow the AbstractChestedHorse fields.
            if llama {
                crate::mobs::llama::to_nbt(m, map);
            } else if species == S::SkeletonHorse {
                crate::mobs::riders::write_nbt(m, map);
            }
        }
        "minecraft:bat" => {
            // Bat.addAdditionalSaveData: the whole DATA_ID_FLAGS byte.
            let flags = m.extra.bat_flags | u8::from(m.extra.flag);
            map.insert("BatFlags".into(), Tag::Byte(flags as i8));
        }
        "minecraft:turtle" => {
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            }
            map.insert("has_egg".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        "minecraft:creaking" => {
            // home_pos: the heart it is bound to.
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            } else {
                map.remove(&"home_pos".into());
            }
        }
        "minecraft:zombie_villager" => {
            map.insert("VillagerData".into(), villager_data(m));
            crate::mobs::zombie_villager::write_nbt(m, map, items);
        }
        "minecraft:villager" => {
            crate::mobs::villager::write_nbt(m, map, items);
            put_memories(map, m);
        }
        // Dolphin.addAdditionalSaveData: the fish it was fed, and how wet it is.
        "minecraft:dolphin" => {
            map.insert("GotFish".into(), Tag::Byte(i8::from(m.extra.flag)));
            map.insert("Moistness".into(), Tag::Int(m.extra.counter));
        }
        "minecraft:wandering_trader" => {
            // WanderingTrader.addAdditionalSaveData: the visit's countdown and
            // the place it is walking to, a BlockPos codec (an int array).
            map.insert("DespawnDelay".into(), Tag::Int(m.extra.ints[5]));
            if let Some(p) = m.extra.point {
                // The trader keeps its target as a position; the tag is a
                // BlockPos, so only the block it is in is saved.
                map.insert(
                    "wander_target".into(),
                    Tag::IntArray(vec![
                        p[0].floor() as i32,
                        p[1].floor() as i32,
                        p[2].floor() as i32,
                    ]),
                );
            }
        }
        "minecraft:axolotl" => {
            write_axolotl_memories(m, map);
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
            map.insert(
                "FromBucket".into(),
                Tag::Byte(i8::from(m.extra.from_bucket)),
            );
        }
        "minecraft:parrot" => {
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
        }
        "minecraft:salmon" => {
            let size = SALMON_SIZES[usize::from(m.extra.variant).min(2)];
            map.insert("type".into(), string(size));
        }
        // Pufferfish.addAdditionalSaveData: the puff state (0 small, 1 mid,
        // 2 full).
        "minecraft:pufferfish" => {
            map.insert("PuffState".into(), Tag::Int(m.extra.size));
        }
        // TropicalFish.addAdditionalSaveData: the packed variant.
        "minecraft:tropical_fish" => {
            let pattern = m.extra.variant.min(11);
            let packed = i32::from(pattern >= 6)
                | i32::from(pattern % 6) << 8
                | i32::from(m.extra.variant2 & 15) << 16
                | i32::from(m.extra.variant3 & 15) << 24;
            map.insert("Variant".into(), Tag::Int(packed));
        }
        "minecraft:allay" => {
            // InventoryCarrier: the fetched stack; the liked player memory.
            let inv = crate::mobs::allay::inventory(m)
                .map(|stack| item_to_nbt(stack, items))
                .into_iter()
                .collect();
            map.insert("Inventory".into(), Tag::List(inv));
            if let Some(st) = &m.extra.allay {
                map.insert("listener".into(), st.vibration.to_nbt());
            }
            map.insert(
                "DuplicationCooldown".into(),
                Tag::Long(i64::from(m.extra.egg_time)),
            );
            let mut memories: Vec<(&str, Tag)> = Vec::new();
            if m.extra.tame {
                // `MemorySlot.visit`: the time to live only when it can expire.
                let mut liked = vec![(
                    "value",
                    Tag::IntArray(m.extra.owner.unwrap_or([0, 0, 0, 1]).to_vec()),
                )];
                if let Some(ttl) = m.extra.allay.as_ref().and_then(|st| st.liked_player_ttl) {
                    liked.push(("ttl", Tag::Long(ttl)));
                }
                memories.push(("minecraft:liked_player", Tag::compound(liked)));
            }
            // GlobalPos.CODEC, and the cooldown while it runs.
            if let Some(liked) = m.extra.allay.as_ref().and_then(|st| st.noteblock) {
                memories.push((
                    "minecraft:liked_noteblock",
                    Tag::compound([(
                        "value",
                        Tag::compound([
                            ("dimension", string(liked.dimension)),
                            ("pos", Tag::IntArray(liked.pos.to_vec())),
                        ]),
                    )]),
                ));
            }
            if m.extra.ints[2] > 0 {
                memories.push((
                    "minecraft:liked_noteblock_cooldown_ticks",
                    Tag::compound([("value", Tag::Int(m.extra.ints[2]))]),
                ));
            }
            // ITEM_PICKUP_COOLDOWN_TICKS (`Codec.INT`), which a throw sets to 60,
            // while the memory is there (its value 0 included).
            if let Some(ticks) = crate::mobs::allay::pickup_cooldown(m) {
                memories.push((
                    "minecraft:item_pickup_cooldown_ticks",
                    Tag::compound([("value", Tag::Int(ticks))]),
                ));
            }
            // Always written, so a forgotten note block does not come back
            // from the compound the allay was loaded with.
            map.insert(
                "Brain".into(),
                Tag::compound([("memories", Tag::compound(memories))]),
            );
        }
        "minecraft:nautilus" | "minecraft:zombie_nautilus" => {
            crate::mobs::nautilus::to_nbt(m, map, variants);
        }
        "minecraft:sulfur_cube" => crate::mobs::sulfur_cube::to_nbt(m, map),
        "minecraft:bee" => crate::mobs::bee::to_nbt(m, map, &mut uuid_of),
        "minecraft:sniffer" => crate::mobs::sniffer::to_nbt(m, map),
        "minecraft:wither" => {
            map.insert("Invul".into(), Tag::Int(m.extra.counter));
        }
        // IronGolem.addAdditionalSaveData.
        "minecraft:iron_golem" => {
            map.insert(
                "PlayerCreated".into(),
                Tag::Byte(i8::from(m.extra.player_created)),
            );
            write_persistent_anger(m, map, &mut uuid_of);
        }
        "minecraft:polar_bear" | "minecraft:zombified_piglin" => {
            write_persistent_anger(m, map, &mut uuid_of);
        }
        // SnowGolem.addAdditionalSaveData: whether it still wears its pumpkin.
        "minecraft:snow_golem" => {
            map.insert("Pumpkin".into(), Tag::Byte(i8::from(!m.extra.sheared)));
        }
        // `Warden.addAdditionalSaveData`: the anger - one `{uuid, anger}` pair
        // per suspect - and the vibration listener, and nothing else. A warden
        // is no `NeutralMob`, so there is no `angry_at` or `anger_end_time`: the
        // entity a suspect is is named by its UUID, not by a live id.
        "minecraft:warden" => {
            if let Some(st) = &m.extra.warden {
                map.insert("anger".into(), warden_anger_to_nbt(&st.anger, &mut uuid_of));
                map.insert("listener".into(), st.vibration.to_nbt());
            }
            write_warden_owned_memories(m, map);
        }
        // Armadillo.addAdditionalSaveData: the time left to the next scute, and
        // the state it is in, which is always written (`idle` when it is not
        // rolled up).
        "minecraft:armadillo" => {
            map.insert(
                "state".into(),
                string(crate::mobs::armadillo::state_name(
                    crate::mobs::armadillo::state(m),
                )),
            );
            map.insert("scute_time".into(), Tag::Int(m.extra.egg_time));
        }
        "minecraft:ravager" => {
            map.insert("AttackTick".into(), Tag::Int(m.extra.counter));
            map.insert("StunTick".into(), Tag::Int(m.extra.counter2));
            map.insert("RoarTick".into(), Tag::Int(m.extra.anim_ticks));
        }
        "minecraft:vex" => crate::mobs::vex::write_nbt(m, map, &mut uuid_of),
        "minecraft:endermite" => {
            map.insert("Lifetime".into(), Tag::Int(m.extra.counter));
        }
        // Enderman.addAdditionalSaveData: the block it carries, saved only
        // while it carries one.
        "minecraft:enderman" => {
            match (m.extra.carried, registry) {
                (Some(state), Some(registry)) => {
                    map.insert("carriedBlockState".into(), state_to_nbt(registry, state));
                }
                // Nothing carries a block state out without the registry, and a
                // caller without one keeps what the loaded compound had, so the
                // key is only dropped when a registry could have written it.
                (_, None) => map.retain("carriedBlockState"),
                (None, Some(_)) => {
                    map.remove(&"carriedBlockState".into());
                }
            }
            write_persistent_anger(m, map, &mut uuid_of);
        }
        "minecraft:shulker" => {
            map.insert("AttachFace".into(), Tag::Byte(m.extra.variant3 as i8));
            map.insert("Peek".into(), Tag::Byte(m.extra.counter as i8));
            map.insert("Color".into(), Tag::Byte(m.extra.color as i8));
        }
        // HappyGhast.addAdditionalSaveData: the still timeout (`serverStillTimeout`,
        // kept in `counter`), which vanilla writes even at 0.
        "minecraft:happy_ghast" => {
            map.insert("still_timeout".into(), Tag::Int(m.extra.counter));
        }
        // GlowSquid.addAdditionalSaveData.
        "minecraft:glow_squid" => {
            map.insert(
                "DarkTicksRemaining".into(),
                Tag::Int(crate::mobs::aquatic::dark_ticks(m)),
            );
        }
        // Phantom.addAdditionalSaveData: the size, and the circling anchor.
        // `storeNullable` writes nothing for a phantom that has none, so its
        // save carries no `anchor_pos` at all.
        "minecraft:phantom" => {
            match m.extra.home {
                Some(anchor) => {
                    map.insert("anchor_pos".into(), Tag::IntArray(anchor.to_vec()));
                }
                None => {
                    map.remove(&"anchor_pos".into());
                }
            }
            map.insert("size".into(), Tag::Int(m.extra.size));
        }
        "minecraft:slime" | "minecraft:magma_cube" => {
            map.insert("Size".into(), Tag::Int(m.extra.size.max(1) - 1));
            map.insert("wasOnGround".into(), Tag::Byte(i8::from(m.extra.flag2)));
        }
        "minecraft:skeleton" => {
            map.insert("StrayConversionTime".into(), Tag::Int(conversion_time(m)));
            let time = if m.in_powder_snow {
                m.extra.counter
            } else {
                -1
            };
            map.insert("FreezingTime".into(), Tag::Int(time));
        }
        "minecraft:zoglin" => {
            map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
        }
        // Sheep.addAdditionalSaveData: its wool as it is now.
        "minecraft:sheep" => {
            map.insert("Sheared".into(), Tag::Byte(i8::from(m.extra.sheared)));
            map.insert("Color".into(), Tag::Byte((m.extra.color & 15) as i8));
        }
        // CopperGolem.addAdditionalSaveData.
        "minecraft:copper_golem" => {
            map.insert(
                "next_weather_age".into(),
                Tag::Long(m.extra.vector[0] as i64),
            );
            map.insert(
                "weather_state".into(),
                string(crate::mobs::copper_golem::WEATHER[m.extra.variant.min(3) as usize]),
            );
        }
        "minecraft:ghast" => crate::mobs::nether::to_nbt(m, map),
        _ => {}
    }
    if m.species() == S::Frog {
        crate::mobs::brain::write_frog(m, map);
    }
    // Bogged.addAdditionalSaveData: whether its mushrooms were sheared.
    if m.species() == S::Bogged {
        map.insert("sheared".into(), Tag::Byte(i8::from(m.extra.sheared)));
    }
    // SpellcasterIllager.addAdditionalSaveData: save the live casting timer,
    // including zero and negative values, rather than stale loaded NBT.
    if matches!(m.species(), S::Evoker | S::Illusioner) {
        map.insert("SpellTicks".into(), Tag::Int(m.extra.counter));
    }
    // Vindicator.addAdditionalSaveData: `Johnny` only while the flag is set
    // (`if (this.isJohnny) output.putBoolean("Johnny", true)`).
    if m.species() == S::Vindicator {
        if m.extra.johnny {
            map.insert("Johnny".into(), Tag::Byte(1));
        } else {
            map.remove(&"Johnny".into());
        }
    }
    if m.death_canceled {
        map.insert("sunrise:death_canceled".into(), Tag::Byte(1));
    }
}
/// `NeutralMob.addPersistentAngerSaveData` at the class's own save slot.
fn write_persistent_anger(
    m: &Mob,
    map: &mut EntityTagWriter,
    uuid_of: &mut impl FnMut(u32) -> Option<[i32; 4]>,
) {
    map.insert(
        "anger_end_time".into(), // Keep the absolute value even after the time has passed.
        Tag::Long(m.extra.anger_end),
    );
    // `angry_at` only while there is an anger target: the UUID of the one
    // the level has (`uuid_of`). A mob target read from the save that is
    // not in the world (`Who::Mob(0)`) keeps the loaded UUID.
    match m.extra.angry_at {
        Some(crate::mobs::Who::Mob(0)) => map.retain("angry_at"),
        target => match target.and_then(|who| who_uuid(who, uuid_of)) {
            Some(uuid) => {
                map.insert("angry_at".into(), Tag::IntArray(uuid.to_vec()));
            }
            None => {
                map.remove(&"angry_at".into());
            }
        },
    }
}
