//! Species-specific entity tracker metadata.
use crate::game::villager_metadata::unhappy_counter;
use sunrise_level::effects::ShownEffect;
use sunrise_protocol::entity_data::{Particles, Value, serializer};
use sunrise_sim::{
    lan::{
        entity::avatar_main_arm_meta,
        hub::{Hub, stuck_meta},
    },
    mobs::{Extra, Mob, Species, Who},
};

/// The SynchedEntityData index of the baby flag: AgeableMob.DATA_BABY_ID and
/// Zombie.DATA_BABY_ID (Zoglin's too) are the first entry after Mob's (16);
/// Piglin's comes after AbstractPiglin's zombification flag (17). Every
/// AgeableMob has one, the happy ghast and the nautilus included (#6133).
pub fn baby_index(species: Species) -> Option<u8> {
    use Species as S;
    match species {
        S::Piglin => Some(17),
        s if sunrise_sim::mobs::finalize::ageable(s)
            || sunrise_sim::mobs::doors::is_zombie(s)
            || s == S::Zoglin =>
        {
            Some(16)
        }
        _ => None,
    }
}

/// The variant and look entries of `mob_meta`'s mobs (registry holders, the
/// fish's, the bee's, the copper golem's weathering...), in a match of their
/// own: `mob_meta`'s arms hold these mobs' other entries.
fn mob_variant_meta(
    m: &Mob,
    hub: &Hub,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    let (kind, e) = (m.kind.id.as_str(), &m.extra);
    let id = |registry: &str, name: &str| hub.registry_id(registry, &format!("minecraft:{name}"));
    match kind {
        // Axolotl.DATA_VARIANT (after AgeableMob's two).
        "minecraft:axolotl" => {
            if e.variant != 0 {
                meta.push((18, serializer::INT, Value::Int(i32::from(e.variant))));
            }
        }
        "minecraft:pig" | "minecraft:cow" | "minecraft:chicken" => {
            let (v_idx, v_ser, v_reg, s_idx, s_ser, s_reg, v_id, s_id) = match kind {
                "minecraft:pig" => (
                    19,
                    serializer::PIG_VARIANT,
                    "minecraft:pig_variant",
                    20,
                    serializer::PIG_SOUND_VARIANT,
                    "minecraft:pig_sound_variant",
                    e.pig_variant.map(|r| r.network_id() as i32),
                    e.pig_sound_variant.map(|r| r.network_id() as i32),
                ),
                "minecraft:cow" => (
                    18,
                    serializer::COW_VARIANT,
                    "minecraft:cow_variant",
                    19,
                    serializer::COW_SOUND_VARIANT,
                    "minecraft:cow_sound_variant",
                    e.cow_variant.map(|r| r.network_id() as i32),
                    e.cow_sound_variant.map(|r| r.network_id() as i32),
                ),
                _ => (
                    18,
                    serializer::CHICKEN_VARIANT,
                    "minecraft:chicken_variant",
                    19,
                    serializer::CHICKEN_SOUND_VARIANT,
                    "minecraft:chicken_sound_variant",
                    e.chicken_variant.map(|r| r.network_id() as i32),
                    e.chicken_sound_variant.map(|r| r.network_id() as i32),
                ),
            };
            let default_v = id(v_reg, "temperate");
            if let Some(v) = v_id.or(default_v)
                && Some(v) != default_v
            {
                meta.push((v_idx, v_ser, Value::Id(v)));
            }
            if let Some(v) = default_v {
                defaults.push((v_idx, v_ser, Value::Id(v)));
            }
            let default_s = id(s_reg, "classic");
            if let Some(v) = s_id.or(default_s)
                && Some(v) != default_s
            {
                meta.push((s_idx, s_ser, Value::Id(v)));
            }
            if let Some(v) = default_s {
                defaults.push((s_idx, s_ser, Value::Id(v)));
            }
        }
        // Frog.DATA_VARIANT_ID: the world table's network id directly (`RegistryRef<FrogVariant>`).
        "minecraft:frog" => {
            let default_id = id("minecraft:frog_variant", "temperate");
            let variant_id = e.frog_variant.map(|r| r.network_id() as i32).or(default_id);
            if let Some(v) = variant_id
                && Some(v) != default_id
            {
                meta.push((18, serializer::FROG_VARIANT, Value::Id(v)));
            }
            if let Some(v) = default_id {
                defaults.push((18, serializer::FROG_VARIANT, Value::Id(v)));
            }
        }
        // ZombieNautilus.DATA_VARIANT_ID carries the world table's network id
        // directly (`RegistryRef<ZombieNautilusVariant>`).
        "minecraft:zombie_nautilus" => {
            let default = id("minecraft:zombie_nautilus_variant", "temperate");
            let variant_id = e
                .zombie_nautilus_variant
                .map(|r| r.network_id() as i32)
                .or(default);
            if let Some(v) = variant_id
                && Some(v) != default
            {
                meta.push((21, serializer::ZOMBIE_NAUTILUS_VARIANT, Value::Id(v)));
            }
            if let Some(v) = default {
                defaults.push((21, serializer::ZOMBIE_NAUTILUS_VARIANT, Value::Id(v)));
            }
        }
        // Rabbit.DATA_TYPE_ID (after AgeableMob's two) and
        // Parrot.DATA_VARIANT_ID (after TamableAnimal's two and its flags and
        // owner). The fox's type goes with its flags (#2538).
        "minecraft:rabbit" | "minecraft:parrot" => {
            let index = if kind == "minecraft:parrot" { 20 } else { 18 };
            if e.variant != 0 {
                meta.push((index, serializer::INT, Value::Int(i32::from(e.variant))));
            }
        }
        // Bee.DATA_FLAGS_ID (rolling 2, stung 4, nectar 8) and
        // DATA_ANGER_END_TIME: the texture shows the nectar and the anger.
        "minecraft:bee" => {
            let bee = sunrise_sim::mobs::bee::get(m);
            let flags = bee.map_or(0, |b| b.flags);
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags as i8)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(0)));
            let anger_end = bee.map_or(-1, |b| b.anger_end);
            if anger_end != -1 {
                meta.push((19, serializer::LONG, Value::Long(anger_end)));
            }
            defaults.push((19, serializer::LONG, Value::Long(-1)));
        }
        // AbstractFish.FROM_BUCKET takes 16: Salmon.DATA_TYPE (small 0,
        // medium 1, large 2; medium is the data's default, so always sent),
        // Pufferfish.PUFF_STATE and TropicalFish.DATA_ID_TYPE_VARIANT: the
        // pattern's packed id (size, then index << 8), base colour << 16 and
        // pattern colour << 24.
        "minecraft:salmon" => {
            meta.push((17, serializer::INT, Value::Int(i32::from(e.variant.min(2)))));
        }
        "minecraft:pufferfish" => {
            if e.size > 0 {
                meta.push((17, serializer::INT, Value::Int(e.size)));
            }
            defaults.push((17, serializer::INT, Value::Int(0)));
        }
        "minecraft:tropical_fish" => {
            let pattern = e.variant.min(11);
            let packed = i32::from(pattern >= 6)
                | i32::from(pattern % 6) << 8
                | i32::from(e.variant2 & 15) << 16
                | i32::from(e.variant3 & 15) << 24;
            if packed != 0 {
                meta.push((17, serializer::INT, Value::Int(packed)));
            }
        }
        // Tadpole.AGE_LOCKED (after AbstractFish's FROM_BUCKET).
        "minecraft:tadpole" => {
            if e.age_locked {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
        }
        // CopperGolem.DATA_WEATHER_STATE: unaffected 0 to oxidized 3, and it
        // goes back down when the golem is scraped.
        "minecraft:copper_golem" => {
            if e.variant != 0 {
                meta.push((
                    16,
                    serializer::WEATHERING_COPPER_STATE,
                    Value::Id(i32::from(e.variant.min(3))),
                ));
            }
            defaults.push((16, serializer::WEATHERING_COPPER_STATE, Value::Id(0)));
        }
        // Ghast.DATA_IS_CHARGING: the shooting texture.
        "minecraft:ghast" => {
            if e.flag {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
        }
        // WitherBoss.DATA_ID_INV (after the three head targets): the
        // invulnerable ticks, the blinking texture and the growing scale.
        "minecraft:wither" => {
            if e.counter > 0 {
                meta.push((19, serializer::INT, Value::Int(e.counter)));
            }
            defaults.push((19, serializer::INT, Value::Int(0)));
        }
        _ => {}
    }
}

/// A mob's synched data as ServerEntity sends it (only the values that differ
/// from the defaults the client constructs), with vanilla's entry indices:
/// Entity 0..=7, LivingEntity 8..=14, Mob 15, then the subclasses.
/// `defaults` gets the defaults of the entries that can go back to theirs.
pub fn mob_meta(
    m: &Mob,
    hub: &Hub,
    player_entity: Option<i32>,
    game_time: i64,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    // The entries are the class's: a content mod's living type has its
    // template's.
    let kind = m.kind.class_id();
    // Entity.DATA_SHARED_FLAGS_ID: on fire 0x01, sprinting 0x08 (a dashing
    // cat or ocelot), swimming 0x10 (a drowned under water,
    // `Drowned.setSwimming`), invisible 0x20 (the effect's flag) and glowing
    // 0x40 (LivingEntity.updateGlowingStatus).
    let shared = i8::from(m.is_on_fire())
        | i8::from(m.extra.sprinting) << 3
        | i8::from(m.species() == Species::Drowned && m.extra.flag) << 4
        | i8::from(m.is_invisible()) << 5
        | i8::from(m.is_currently_glowing()) << 6;
    if shared != 0 {
        meta.push((0, serializer::BYTE, Value::Byte(shared)));
    }
    // Entity.DATA_POSE: the pose the species keeps in its own fields (a
    // pose back at standing goes out by the entity's own default).
    let pose = sunrise_sim::mobs::sync::pose_id(m);
    if pose != 0 {
        meta.push((6, serializer::POSE, Value::Id(i32::from(pose))));
    }
    // LivingEntity.DATA_SLEEPING_POS_ID: the bed a sleeping villager lies in.
    if let Some(bed) = sunrise_sim::mobs::villager::sleeping_pos(m) {
        meta.push((14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(Some(bed))));
    }
    if let Some(name) = &m.custom_name {
        meta.push((
            2,
            serializer::OPTIONAL_COMPONENT,
            Value::OptText(Some(crate::game::name_component::name_component(m, name))),
        ));
    }
    if m.name_visible {
        meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
    }
    // A name taken away or hidden again is sent as none / false.
    defaults.push((2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)));
    defaults.push((3, serializer::BOOLEAN, Value::Bool(false)));
    // Entity.DATA_NO_GRAVITY (`setNoGravity`: `/data`, a summon's NoGravity, a mod).
    if m.no_gravity {
        meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
    }
    defaults.push((5, serializer::BOOLEAN, Value::Bool(false)));
    // A witch drinking and a piglin admiring do not use the active hand.
    let uses_item = sunrise_sim::mobs::spear_use::is_using_item(m);
    let using = i8::from(uses_item) | i8::from(m.extra.using_off_hand) << 1;
    if using != 0 {
        meta.push((8, serializer::BYTE, Value::Byte(using)));
    }
    defaults.push((8, serializer::BYTE, Value::Byte(0)));
    // LivingEntity.DATA_HEALTH_ID: defined as 1, then set to the max health.
    let health = m.health.max(0.0);
    if health != 1.0 {
        meta.push((9, serializer::FLOAT, Value::Float(health)));
    }
    defaults.push((9, serializer::FLOAT, Value::Float(1.0)));
    // LivingEntity.DATA_EFFECT_PARTICLES and DATA_EFFECT_AMBIENCE_ID: the
    // swirls of its effects, or what the native mods made of them
    // (`Game::lan_effect_particles`, kept only while one listens: mods are
    // installed once per process).
    match &m.extra.modified_effect_particles {
        Some(modified) => meta.extend_from_slice(&modified.meta),
        None => meta.extend(hub.effect_meta(mob_effects(m))),
    }
    defaults.push((
        10,
        serializer::PARTICLES,
        Value::Particles(Particles::new()),
    ));
    defaults.push((11, serializer::BOOLEAN, Value::Bool(false)));
    // LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID.
    stuck_meta(
        meta,
        defaults,
        (m.extra.stuck.arrows, m.extra.stuck.stingers),
    );
    // Mob.DATA_MOB_FLAGS_ID: no AI 1, left-handed 2, aggressive 4 (what
    // draws a vindicator's axe, a skeleton's drawn bow, a piglin's raised
    // weapon).
    let flags =
        i8::from(m.no_ai) | i8::from(m.extra.left_handed) << 1 | i8::from(m.ai.aggressive) << 2;
    if flags != 0 {
        meta.push((15, serializer::BYTE, Value::Byte(flags)));
    }
    defaults.push((15, serializer::BYTE, Value::Byte(0)));
    // AgeableMob.DATA_BABY_ID (and Zombie's, Zoglin's, Piglin's).
    if let Some(index) = baby_index(m.species()) {
        if m.is_baby() {
            meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((index, serializer::BOOLEAN, Value::Bool(false)));
        // AgeableMob.AGE_LOCKED, the entry after it (not Zombie's or Piglin's).
        if index == 16 && sunrise_sim::mobs::finalize::ageable(m.species()) {
            if m.extra.age_locked {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
        }
    }
    let e = &m.extra;
    let id = |registry: &str, name: &str| hub.registry_id(registry, &format!("minecraft:{name}"));
    // Pig.DATA_BOOST_TIME and Strider.DATA_BOOST_TIME (18, the first entry
    // after AgeableMob's two). The total `boost()` stored in `counter2`.
    // The default 0 is not registered: vanilla never writes it back, and the
    // rider's client finishes the timer `onSynced` started.
    if matches!(kind, "minecraft:pig" | "minecraft:strider") && e.counter2 > 0 {
        meta.push((18, serializer::INT, Value::Int(e.counter2)));
    }
    mob_variant_meta(m, hub, meta, defaults);
    mob_raider_meta(m, meta, defaults);
    match kind {
        "minecraft:guardian" | "minecraft:elder_guardian" => {
            // Guardian.DATA_ID_MOVING and DATA_ID_ATTACK_TARGET.
            if e.flag2 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            let target = if e.flag {
                m.ai.target.and_then(|who| match who {
                    Who::Player => player_entity,
                    Who::Mob(id) => Some(id as i32),
                })
            } else {
                None
            };
            if let Some(target) = target {
                meta.push((17, serializer::INT, Value::Int(target)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
            defaults.push((17, serializer::INT, Value::Int(0)));
        }
        // Bat.DATA_ID_FLAGS: resting in bit 0, flying by default.
        "minecraft:bat" => {
            if e.flag {
                meta.push((16, serializer::BYTE, Value::Byte(1)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(0)));
        }
        // SnowGolem.DATA_PUMPKIN_ID: 16 while the pumpkin is on (the default).
        "minecraft:snow_golem" => {
            if e.sheared {
                meta.push((16, serializer::BYTE, Value::Byte(0)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(16)));
        }
        // MushroomCow.DATA_TYPE (after AgeableMob's two): 0 red, 1 brown.
        "minecraft:mooshroom" => {
            if e.variant == 1 {
                meta.push((18, serializer::INT, Value::Int(1)));
            }
            defaults.push((18, serializer::INT, Value::Int(0)));
        }
        // AbstractPiglin.DATA_IMMUNE_TO_ZOMBIFICATION (16: with the dimension it
        // makes it shake) and Piglin.DATA_IS_DANCING (after the baby flag and
        // the crossbow charge).
        "minecraft:piglin" => {
            if e.flag2 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
            if e.scratch[0] > 0 {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 when sheared.
        "minecraft:sheep" => {
            let value = (e.color & 15) | if e.sheared { 16 } else { 0 };
            if value != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(value as i8)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(0)));
        }
        // Fox.DATA_TYPE_ID and DATA_FLAGS_ID (after AgeableMob's two; sleeping 32, ...).
        "minecraft:fox" => {
            if e.variant != 0 {
                meta.push((18, serializer::INT, Value::Int(i32::from(e.variant))));
            }
            defaults.push((18, serializer::INT, Value::Int(0)));
            let flags = (e.ints[0] & 0xff) as u8 as i8;
            if flags != 0 {
                meta.push((19, serializer::BYTE, Value::Byte(flags)));
            }
            defaults.push((19, serializer::BYTE, Value::Byte(0)));
        }
        "minecraft:parrot" => {
            // TamableAnimal.DATA_FLAGS_ID: in the sitting pose 1, tame 4.
            let flags = i8::from(e.sitting_pose) | if e.tame { 4 } else { 0 };
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags)));
            }
        }
        "minecraft:cat" | "minecraft:wolf" => {
            // TamableAnimal.DATA_FLAGS_ID: in the sitting pose 1, tame 4.
            let flags = i8::from(e.sitting_pose) | if e.tame { 4 } else { 0 };
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags)));
            }
            if kind == "minecraft:cat" {
                // Cat.DATA_VARIANT_ID and DATA_SOUND_VARIANT_ID carry the world
                // table's network id directly (`RegistryRef<K>`).
                if let Some(r) = e.cat_variant {
                    meta.push((
                        20,
                        serializer::CAT_VARIANT,
                        Value::Id(r.network_id() as i32),
                    ));
                }
                if let Some(r) = e.cat_sound_variant {
                    meta.push((
                        24,
                        serializer::CAT_SOUND_VARIANT,
                        Value::Id(r.network_id() as i32),
                    ));
                }
            } else {
                // Wolf.DATA_VARIANT_ID and DATA_SOUND_VARIANT_ID carry the world
                // table's network id directly (`RegistryRef<K>`).
                if let Some(r) = e.wolf_variant {
                    meta.push((
                        23,
                        serializer::WOLF_VARIANT,
                        Value::Id(r.network_id() as i32),
                    ));
                }
                if let Some(r) = e.wolf_sound_variant {
                    meta.push((
                        24,
                        serializer::WOLF_SOUND_VARIANT,
                        Value::Id(r.network_id() as i32),
                    ));
                }
            }
            // Cat.IS_LYING and RELAX_STATE_ONE.
            if kind == "minecraft:cat" {
                for (index, on) in [(21, e.flag), (22, e.relax_one)] {
                    if on {
                        meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
                    }
                    defaults.push((index, serializer::BOOLEAN, Value::Bool(false)));
                }
            }
            // Wolf.DATA_COLLAR_COLOR and Cat's, red by default.
            let collar = if kind == "minecraft:cat" { 23 } else { 21 };
            let red = i32::from(sunrise_sim::mobs::tame::RED);
            if i32::from(e.color & 15) != red {
                meta.push((collar, serializer::INT, Value::Int(i32::from(e.color & 15))));
            }
            defaults.push((collar, serializer::INT, Value::Int(red)));
            // Wolf.DATA_ANGER_END_TIME: the game time the anger ends, -1 when calm.
            if kind == "minecraft:wolf" {
                // Wolf.DATA_INTERESTED_ID: BegGoal's flag, the head tilt.
                if e.flag {
                    meta.push((20, serializer::BOOLEAN, Value::Bool(true)));
                }
                defaults.push((20, serializer::BOOLEAN, Value::Bool(false)));
                if e.anger > 0 {
                    let end = game_time + i64::from(e.anger);
                    meta.push((22, serializer::LONG, Value::Long(end)));
                }
                defaults.push((22, serializer::LONG, Value::Long(-1)));
            }
        }
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:llama"
        | "minecraft:trader_llama"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:camel" => {
            // AbstractHorse.DATA_ID_FLAGS: tame 2, eating 16 (grazing), standing
            // 32 (rearing).
            let grazes = sunrise_sim::mobs::equine::grazes(m.species());
            let flags = if e.tame { 2 } else { 0 }
                | if grazes && e.flag { 16 } else { 0 }
                | if grazes && e.counter > 0 { 32 } else { 0 };
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(0)));
            if kind == "minecraft:horse" {
                let v = i32::from(e.variant) | i32::from(e.variant2) << 8;
                if v != 0 {
                    meta.push((19, serializer::INT, Value::Int(v)));
                }
            } else if kind.ends_with("llama") {
                if e.chested {
                    meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
                }
                meta.push((
                    20,
                    serializer::INT,
                    Value::Int(i32::from(e.variant2.max(1))),
                ));
                if e.variant != 0 {
                    meta.push((21, serializer::INT, Value::Int(i32::from(e.variant))));
                }
            } else if (kind == "minecraft:donkey" || kind == "minecraft:mule") && e.chested {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        // HappyGhast.STAYS_STILL (after AgeableMob's two and IS_LEASH_HOLDER),
        // set while `serverStillTimeout > 0` (`syncStayStillFlag`).
        "minecraft:happy_ghast" => {
            // HappyGhast.IS_LEASH_HOLDER (`setLeashHolder` in `tick`).
            if sunrise_sim::mobs::happy_ghast::is_leash_holder(m) {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
            if sunrise_sim::mobs::happy_ghast::on_still_timeout(m) {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Villager.DATA_VILLAGER_DATA and ZombieVillager's (after Zombie's three).
        "minecraft:villager" | "minecraft:zombie_villager" => {
            use sunrise_sim::mobs::villager::VILLAGER_TYPES;
            if kind == "minecraft:villager" {
                unhappy_counter(m, meta, defaults);
            } else {
                // ZombieVillager.DATA_CONVERTING_ID: the cure's shaking.
                if sunrise_sim::mobs::zombie_villager::is_converting(m) {
                    meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
                }
                defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
            }
            let villager_type = VILLAGER_TYPES[e.variant as usize % VILLAGER_TYPES.len()];
            let profession =
                sunrise_sim::mobs::villager::profession_id(e.profession.index().unwrap_or(0));
            if let (Some(t), Some(p)) = (
                id("minecraft:villager_type", villager_type),
                id("minecraft:villager_profession", profession),
            ) {
                let index = if kind == "minecraft:villager" { 19 } else { 20 };
                let level = e.villager_level.max(1);
                meta.push((
                    index,
                    serializer::VILLAGER_DATA,
                    Value::Ids(vec![t, p, level]),
                ));
            }
        }
        // Phantom.ID_SIZE (after Mob's flags).
        "minecraft:phantom" => {
            if e.size > 0 {
                meta.push((16, serializer::INT, Value::Int(e.size)));
            }
            defaults.push((16, serializer::INT, Value::Int(0)));
        }
        // AbstractCubeMob.ID_SIZE; SulfurCube then syncs MAX_FUSE (-1 until primed;
        // clients tick it for the swell / white flash) and FROM_BUCKET (#13721).
        "minecraft:slime" | "minecraft:magma_cube" | "minecraft:sulfur_cube" => {
            if e.size > 1 {
                meta.push((18, serializer::INT, Value::Int(e.size)));
            }
            defaults.push((18, serializer::INT, Value::Int(1)));
            if kind == "minecraft:sulfur_cube" {
                // The synced MAX_FUSE, not the countdown: a late tracker gets the value the
                // fuse was set from, and only a change of it reaches those already tracking.
                let max = sunrise_sim::mobs::sulfur_cube::max_fuse(m);
                if max != -1 {
                    meta.push((19, serializer::INT, Value::Int(max)));
                }
                defaults.push((19, serializer::INT, Value::Int(-1)));
                if e.scratch[3] != 0 {
                    meta.push((20, serializer::BOOLEAN, Value::Bool(true)));
                }
                defaults.push((20, serializer::BOOLEAN, Value::Bool(false)));
            }
        }
        // Creeper.DATA_SWELL_DIR, DATA_IS_POWERED and DATA_IS_IGNITED: the
        // guest swells the creeper from them. The direction always goes in,
        // as only changed entries are sent and it drops back to -1.
        "minecraft:creeper" => {
            meta.push((16, serializer::INT, Value::Int(e.counter)));
            if e.charged {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            if e.flag {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        // Pillager.IS_CHARGING_CROSSBOW: the charge pose and the held
        // item's loading animation.
        "minecraft:pillager" => {
            if e.flag {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Panda: UNHAPPY_COUNTER, SNEEZE_COUNTER and EAT_COUNTER (after
        // AgeableMob's two), MAIN_GENE_ID, HIDDEN_GENE_ID and DATA_ID_FLAGS;
        // the client animates the states from them.
        "minecraft:panda" => {
            let (flags, counters) = sunrise_sim::mobs::panda::get(m)
                .map_or((0, [0; 3]), |s| (s.flags, [s.unhappy, s.sneeze, s.eat]));
            for (index, value) in (18..).zip(counters) {
                if value != 0 {
                    meta.push((index, serializer::INT, Value::Int(value)));
                }
                defaults.push((index, serializer::INT, Value::Int(0)));
            }
            for (index, gene) in [(21, e.variant), (22, e.variant2)] {
                if gene != 0 {
                    meta.push((index, serializer::BYTE, Value::Byte(gene as i8)));
                }
                defaults.push((index, serializer::BYTE, Value::Byte(0)));
            }
            if flags != 0 {
                meta.push((23, serializer::BYTE, Value::Byte(flags as i8)));
            }
            defaults.push((23, serializer::BYTE, Value::Byte(0)));
        }
        // PolarBear.DATA_STANDING_ID (after AgeableMob's two): the bear
        // rears up; the client animates it.
        "minecraft:polar_bear" => {
            if e.flag {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // SpellcasterIllager.DATA_SPELL_CASTING_ID: the casting pose.
        "minecraft:evoker" | "minecraft:illusioner" => {
            if e.variant2 != 0 {
                meta.push((17, serializer::BYTE, Value::Byte(e.variant2 as i8)));
            }
            defaults.push((17, serializer::BYTE, Value::Byte(0)));
        }
        // Warden.CLIENT_ANGER_LEVEL: the heartbeat's pace.
        "minecraft:warden" => {
            let anger = e.warden.as_ref().map_or(0, |w| w.client_anger);
            if anger != 0 {
                meta.push((16, serializer::INT, Value::Int(anger)));
            }
            defaults.push((16, serializer::INT, Value::Int(0)));
        }
        // Sniffer.DATA_STATE (after AgeableMob's two).
        "minecraft:sniffer" => {
            if e.variant != 0 {
                meta.push((
                    18,
                    serializer::SNIFFER_STATE,
                    Value::Id(i32::from(e.variant)),
                ));
            }
            defaults.push((18, serializer::SNIFFER_STATE, Value::Id(0)));
        }
        // Shulker.DATA_ATTACH_FACE_ID, DATA_PEEK_ID and DATA_COLOR_ID (16
        // for none).
        "minecraft:shulker" => {
            if e.variant3 != 0 {
                meta.push((16, serializer::DIRECTION, Value::Id(i32::from(e.variant3))));
            }
            defaults.push((16, serializer::DIRECTION, Value::Id(0)));
            if e.counter != 0 {
                meta.push((17, serializer::BYTE, Value::Byte(e.counter as i8)));
            }
            defaults.push((17, serializer::BYTE, Value::Byte(0)));
            if e.color != 16 {
                meta.push((18, serializer::BYTE, Value::Byte(e.color as i8)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(16)));
        }
        // Vex.DATA_FLAGS_ID: charging in bit 0 (the arms and the glowing
        // texture).
        "minecraft:vex" => {
            if e.flag {
                meta.push((16, serializer::BYTE, Value::Byte(1)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(0)));
        }
        // GlowSquid.DATA_DARK_TICKS_REMAINING (after AgeableMob's two): the
        // dimming a hit gave, counted down by the server and the client alike.
        "minecraft:glow_squid" => {
            let ticks = sunrise_sim::mobs::aquatic::dark_ticks(m);
            if ticks != 0 {
                meta.push((18, serializer::INT, Value::Int(ticks)));
            }
            defaults.push((18, serializer::INT, Value::Int(0)));
        }
        // Ocelot.DATA_TRUSTING (after AgeableMob's two).
        "minecraft:ocelot" => {
            if e.trusting {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Mannequin inherits Avatar's main arm. Its own DATA_PROFILE, DATA_IMMOVABLE
        // and DATA_DESCRIPTION follow the Avatar entries; `updateDescription`
        // keeps the last as a component, or none when hidden.
        "minecraft:mannequin" => {
            avatar_main_arm_meta(e.main_hand_left, meta, defaults);
            if let Some(profile) = &m.profile {
                meta.push((
                    17,
                    serializer::RESOLVABLE_PROFILE,
                    Value::Profile(profile.clone()),
                ));
            }
            if e.immovable {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
            let default = sunrise_sim::mobs::MannequinDescription::Default.component();
            let shown = e.description.component();
            if shown != default {
                meta.push((19, serializer::OPTIONAL_COMPONENT, Value::OptText(shown)));
            }
            defaults.push((19, serializer::OPTIONAL_COMPONENT, Value::OptText(default)));
        }
        // Creaking.CAN_MOVE, IS_ACTIVE and IS_TEARING_DOWN.
        "minecraft:creaking" => {
            if e.flag2 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(false)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(true)));
            if e.flag {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
            if sunrise_sim::mobs::creaking::tearing_down(m) {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Allay.DATA_DANCING and DATA_CAN_DUPLICATE (the held item is its
        // main hand equipment).
        "minecraft:allay" => {
            if e.flag {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
            if e.egg_time > 0 {
                meta.push((17, serializer::BOOLEAN, Value::Bool(false)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(true)));
        }
        // Axolotl.DATA_PLAYING_DEAD (after DATA_VARIANT).
        "minecraft:axolotl" => {
            if sunrise_sim::mobs::axolotl::is_playing_dead(m) {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // CopperGolem.COPPER_GOLEM_STATE (after DATA_WEATHER_STATE).
        "minecraft:copper_golem" => {
            if e.variant3 != 0 {
                meta.push((
                    17,
                    serializer::COPPER_GOLEM_STATE,
                    Value::Id(i32::from(e.variant3)),
                ));
            }
            defaults.push((17, serializer::COPPER_GOLEM_STATE, Value::Id(0)));
        }
        // Armadillo.ARMADILLO_STATE (after AgeableMob's two).
        "minecraft:armadillo" => {
            let state = sunrise_sim::mobs::armadillo::state(m);
            if state != 0 {
                meta.push((18, serializer::ARMADILLO_STATE, Value::Id(i32::from(state))));
            }
            defaults.push((18, serializer::ARMADILLO_STATE, Value::Id(0)));
        }
        // Frog.DATA_TONGUE_TARGET_ID (after the variant): the entity id plus
        // one, 0 for none.
        "minecraft:frog" => {
            if let Some(target) = e.frog.tongue_target {
                let id = i32::try_from(target).map_or(0, |t| t + 1);
                meta.push((19, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(id)));
            }
            defaults.push((19, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(0)));
        }
        // Goat.DATA_HAS_LEFT_HORN and DATA_HAS_RIGHT_HORN (after
        // DATA_IS_SCREAMING_GOAT): both true by default.
        "minecraft:goat" => {
            for (index, lost) in [(19, e.lost_horns[0]), (20, e.lost_horns[1])] {
                if lost {
                    meta.push((index, serializer::BOOLEAN, Value::Bool(false)));
                }
                defaults.push((index, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        "minecraft:enderman" => enderman(e, meta, defaults),
        // Zombie.DATA_DROWNED_CONVERSION_ID (after DATA_BABY_ID and
        // DATA_SPECIAL_TYPE_ID): the shaking of the conversion under water.
        "minecraft:zombie" | "minecraft:husk" => {
            if e.counter2 > 0 {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Skeleton.DATA_STRAY_CONVERSION_ID (the first entry after Mob's):
        // the shaking of the freezing conversion.
        "minecraft:skeleton" => {
            if e.counter2 > 0 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Bogged.DATA_SHEARED (the first entry after Mob's): the mushrooms
        // on its head are gone.
        "minecraft:bogged" => {
            if e.sheared {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
        }
        // AbstractVillager.DATA_UNHAPPY_COUNTER (after AgeableMob's two).
        "minecraft:wandering_trader" => unhappy_counter(m, meta, defaults),
        // Hoglin.DATA_IMMUNE_TO_ZOMBIFICATION (after AgeableMob's two) and
        // AbstractPiglin's, the first entry of its subclasses (the piglin's is
        // in its own arm): with the dimension it makes them shake.
        "minecraft:hoglin" | "minecraft:piglin_brute" => {
            let index = if kind == "minecraft:hoglin" { 18 } else { 16 };
            if e.flag2 {
                meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((index, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Blaze.DATA_FLAGS_ID: charged 1, which makes it burn.
        "minecraft:blaze" => {
            if e.flag {
                meta.push((16, serializer::BYTE, Value::Byte(1)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(0)));
        }
        // Turtle.HAS_EGG and LAYING_EGG (after AgeableMob's two).
        "minecraft:turtle" => {
            if e.flag {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            if e.flag2 {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // WitherBoss.DATA_TARGET_B and DATA_TARGET_C (after DATA_TARGET_A, the
        // main target): the entities the side heads look at, by id.
        "minecraft:wither" => {
            for (k, index) in [(0, 17), (1, 18)] {
                let target = match e.ints[k] {
                    0 => None,
                    -1 => player_entity,
                    id => Some(id),
                };
                if let Some(target) = target {
                    meta.push((index, serializer::INT, Value::Int(target)));
                }
                defaults.push((index, serializer::INT, Value::Int(0)));
            }
        }
        _ => {}
    }
    // Camel.DASH and LAST_POSE_CHANGE_TICK (after AbstractHorse's flags).
    if sunrise_sim::mobs::camel::is_camel(m.species()) {
        if e.flag2 {
            meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        let tick = sunrise_sim::mobs::camel::synced_pose_tick(m, game_time);
        if tick != 0 {
            meta.push((20, serializer::LONG, Value::Long(tick)));
        }
        defaults.push((20, serializer::LONG, Value::Long(0)));
    }
}

/// A mob's effects as its swirls are computed from.
pub fn mob_effects(m: &Mob) -> impl Iterator<Item = ShownEffect> + '_ {
    (m.extra.effects.iter()).map(|e| ShownEffect {
        effect: e.id,
        amplifier: e.amplifier,
        visible: !m.extra.hidden_effects.contains(&e.id),
        ambient: m.extra.ambient_effects.contains(&e.id),
    })
}

/// Raider.IS_CELEBRATING, and a witch's Witch.DATA_USING_ITEM (17, after it):
/// `isDrinkingPotion`, set for the drink and cleared when it is done. Both also
/// list false, so tracking sends the end of a celebration or a drink.
pub fn mob_raider_meta(
    m: &Mob,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    use sunrise_sim::mobs::raider;
    if raider::is_raider(m.species()) {
        if raider::state(m).is_some_and(|state| state.celebrating) {
            meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
    }
    if m.species() == Species::Witch {
        if m.extra.flag {
            meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
    }
}

/// What a client shakes a mob by: Entity.DATA_TICKS_FROZEN (7), whose full
/// count is `isFullyFrozen`, and Strider.DATA_SUFFOCATING (19, after
/// AgeableMob's two and DATA_BOOST_TIME).
pub fn mob_freeze_meta(
    m: &Mob,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    // `setTicksFrozen` can go negative (a mob warming up), and a client reads
    // the count as is, so anything but the default 0 goes out.
    if m.ticks_frozen != 0 {
        meta.push((7, serializer::INT, Value::Int(m.ticks_frozen)));
    }
    defaults.push((7, serializer::INT, Value::Int(0)));
    // Every Entity tracks air, including changes back to its species' default.
    let max = m.max_air_supply();
    if m.air != max {
        meta.push((1, serializer::INT, Value::Int(m.air)));
    }
    defaults.push((1, serializer::INT, Value::Int(max)));
    if m.kind.id == "minecraft:strider" {
        if m.extra.flag {
            meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
    }
}

/// Enderman carried block, creepy and stared-at data, including reset defaults.
fn enderman(extra: &Extra, meta: &mut Vec<(u8, i32, Value)>, defaults: &mut Vec<(u8, i32, Value)>) {
    if let Some(state) = extra.carried {
        meta.push((
            16,
            serializer::OPTIONAL_BLOCK_STATE,
            Value::Id(state.0 as i32),
        ));
    }
    if extra.flag {
        meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
    }
    defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
    if extra.flag2 {
        meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
    }
    defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
    defaults.push((16, serializer::OPTIONAL_BLOCK_STATE, Value::Id(0)));
}
