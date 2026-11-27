//! A server's mobs as their synched data has them
//! (ClientPacketListener.handleSetEntityData, then each entity's
//! `onSyncedDataUpdated`): what a client draws from it rather than from its
//! own simulation, which a client of a server doesn't run.
use crate::game::ClientGame;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_protocol::entity_data::{ParticleData, Particles, Value};
use sunrise_protocol::registry::Builtin;
#[cfg(test)]
use sunrise_sim::mobs::villager::VILLAGER_TYPES;
use sunrise_sim::mobs::{
    Mob, Species, Who, armadillo, bat, bee, camel, creaking, equine, finalize, illusioner, panda,
    phantom, raider, sniffer, species::can_be_a_baby, sync, tame, zombie_villager,
};
use sunrise_world::registry::BlockStateId;

/// The values `apply` takes from one SetEntityData packet, by vanilla's entry
/// indices (Entity 0..=7, LivingEntity 8..=14, Mob 15, then the subclasses).
/// `builtin` names the ids of built-in registries.
pub(in crate::game) fn apply(
    m: &mut Mob,
    values: &[(u8, i32, Value)],
    builtin: &Builtin,
    local_entity: i32,
) -> Option<[f64; 3]> {
    let species = m.species();
    let mut stare_sound = None;
    let baby = sunrise_gameplay_common::game::lan_mob_metadata::baby_index(species);
    if &*m.kind.id == "minecraft:player" {
        player_poses(m, values);
    }
    for (index, _, value) in values {
        match (*index, value) {
            // Entity's signed air and frozen counters are synced as-is.
            (1, Value::Int(air)) => m.air = *air,
            (7, Value::Int(frozen)) => m.ticks_frozen = *frozen,
            // Player.DATA_PLAYER_ABSORPTION_ID.
            (17, Value::Float(absorption)) if &*m.kind.id == "minecraft:player" => {
                m.absorption = *absorption;
            }
            // Entity.DATA_POSE: a player's or mannequin's box follows it.
            (6, Value::Id(pose))
                if matches!(&*m.kind.id, "minecraft:player" | "minecraft:mannequin") =>
            {
                m.extra.avatar_pose = sunrise_sim::pose::Pose::from_id(*pose);
            }
            // Entity.DATA_SHARED_FLAGS_ID's FLAG_ONFIRE: Entity.isOnFire on a
            // client, which draws the flames. A client doesn't count the
            // fire down: the flag goes off when the server's fire is out.
            (0, Value::Byte(flags)) => {
                m.fire_ticks = i32::from(flags & 1);
                // Entity.isSprinting: a dashing cat's or ocelot's model.
                m.extra.sprinting = flags & 8 != 0;
                // Drowned.setSwimming: the shared flag of swimming.
                if species == Species::Drowned {
                    m.extra.flag = flags & 16 != 0;
                }
                // The shared flag of invisibility: Entity.isInvisible hides
                // the model (an illusioner shows its mirror images).
                let invisible = flags & 32 != 0;
                if species == Species::Illusioner {
                    illusioner::set_invisible(m, invisible);
                }
                if &*m.kind.id != "minecraft:player" {
                    m.invisible = invisible;
                }
            }
            // Entity.DATA_SILENT: Entity.isSilent, which keeps the sounds
            // a client plays itself quiet (LivingEntity.breakItem's).
            (4, Value::Bool(silent)) => m.silent = *silent,
            // Entity.DATA_POSE: the pose the species animates by.
            (6, Value::Id(pose)) => sync::set_pose(m, (*pose).clamp(0, 17) as u8),
            // LivingEntity.DATA_LIVING_ENTITY_FLAGS: using an item 1, in the
            // off hand 2. Bit 0 starts and stops the use; the client ticks the
            // count itself (a bow's pull follows it). stopUsingItem clears
            // only bit 0, so a stopped off-hand use arrives as 2.
            (8, Value::Byte(flags)) => {
                m.extra.using_off_hand = flags & 2 != 0;
                if flags & 1 == 0 {
                    m.extra.using_item = 0;
                } else if m.extra.using_item == 0 {
                    m.extra.using_item = 1;
                }
            }
            // LivingEntity.DATA_HEALTH_ID: the boss bars and the death
            // animation follow it.
            (9, Value::Float(health)) => m.health = *health,
            // LivingEntity.DATA_EFFECT_PARTICLES and DATA_EFFECT_AMBIENCE_ID:
            // the swirls of its effects (`tickEffects` draws them).
            (10, Value::Particles(list)) => m.extra.effect_particles = swirls(list, builtin),
            (11, Value::Bool(ambient)) => m.extra.effect_ambient = *ambient,
            // LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID: what
            // is stuck in the body, which a player's layers draw.
            (12, Value::Int(count)) => m.extra.stuck.arrows = *count,
            (13, Value::Int(count)) => m.extra.stuck.stingers = *count,
            // Mob.DATA_MOB_FLAGS_ID: no AI 1, left-handed 2, aggressive 4.
            // Mob.isAggressive draws the illagers' and skeletons' attack
            // poses and the vindicator's axe.
            (15, Value::Byte(flags)) => {
                m.no_ai = flags & 1 != 0;
                m.extra.left_handed = flags & 2 != 0;
                m.ai.aggressive = flags & 4 != 0;
            }
            // AgeableMob.isBaby: `canBeABaby` too, so a parrot or a slime
            // sent as one is still drawn grown.
            (i, Value::Bool(b)) if Some(i) == baby => {
                m.extra.age = if *b && can_be_a_baby(species) { -1 } else { 0 };
            }
            // AgeableMob.AGE_LOCKED, the entry after DATA_BABY_ID (not
            // Zombie's or Piglin's).
            (17, Value::Bool(locked)) if baby == Some(16) && finalize::ageable(species) => {
                m.extra.age_locked = *locked
            }
            _ => {}
        }
        match (species, *index, value) {
            (Species::Guardian | Species::ElderGuardian, 16, Value::Bool(moving)) => {
                m.extra.flag2 = *moving;
            }
            (Species::Guardian | Species::ElderGuardian, 17, Value::Int(target)) => {
                // Guardian.onSyncedDataUpdated resets the visual charge clock.
                m.extra.counter2 = 0;
                m.extra.flag = *target != 0;
                m.ai.target = (*target != 0).then_some(if *target == local_entity {
                    Who::Player
                } else {
                    Who::Mob(*target as u32)
                });
            }
            // Panda: the counters, the two genes and DATA_ID_FLAGS (after
            // AgeableMob's two entries).
            (Species::Panda, 18, Value::Int(n)) => panda::set_unhappy(m, *n),
            (Species::Panda, 19, Value::Int(n)) => panda::set_sneeze(m, *n),
            (Species::Panda, 20, Value::Int(n)) => panda::set_eat(m, *n),
            (Species::Panda, 21, Value::Byte(gene)) => m.extra.variant = (*gene).clamp(0, 6) as u8,
            (Species::Panda, 22, Value::Byte(gene)) => m.extra.variant2 = (*gene).clamp(0, 6) as u8,
            (Species::Panda, 23, Value::Byte(flags)) => panda::set_flags(m, *flags as u8),
            // PolarBear.DATA_STANDING_ID: the client animates the rearing.
            (Species::PolarBear, 18, Value::Bool(standing)) => m.extra.flag = *standing,
            // Bat.DATA_ID_FLAGS: resting in bit 0.
            (Species::Bat, 16, Value::Byte(flags)) => bat::set_resting(m, flags & 1 != 0),
            // Fox.DATA_TYPE_ID and DATA_FLAGS_ID.
            (Species::Fox, 18, Value::Int(variant)) => {
                m.extra.variant = (*variant).clamp(0, 1) as u8
            }
            (Species::Fox, 19, Value::Byte(flags)) => m.extra.ints[0] = i32::from(*flags as u8),
            // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 sheared.
            (Species::Sheep, 18, Value::Byte(wool)) => {
                m.extra.color = (*wool & 15) as u8;
                m.extra.sheared = wool & 16 != 0;
            }
            // SnowGolem.DATA_PUMPKIN_ID: 16 while the pumpkin is on.
            (Species::SnowGolem, 16, Value::Byte(flags)) => m.extra.sheared = flags & 16 == 0,
            // Bogged.DATA_SHEARED: the mushrooms on its head are gone.
            (Species::Bogged, 16, Value::Bool(sheared)) => m.extra.sheared = *sheared,
            // Tadpole.AGE_LOCKED (after AbstractFish's FROM_BUCKET).
            (Species::Tadpole, 17, Value::Bool(locked)) => m.extra.age_locked = *locked,
            // MushroomCow.DATA_TYPE (after AgeableMob's DATA_BABY_ID and
            // AGE_LOCKED): 0 red, 1 brown.
            (Species::Mooshroom, 18, Value::Int(variant)) => {
                m.extra.variant = (*variant == 1) as u8
            }
            // Pig.DATA_BOOST_TIME and Strider.DATA_BOOST_TIME (the first entry
            // after AgeableMob's two; the variant and DATA_SUFFOCATING are 19).
            // ItemBasedSteering.onSynced: boosting starts, boostTime is 0, and
            // the total is this int (140 to 980 from boost()).
            (Species::Pig | Species::Strider, 18, Value::Int(total)) => {
                m.extra.counter = 0;
                m.extra.counter2 = *total;
            }
            // Piglin.DATA_IS_DANCING: the dance pose.
            (Species::Piglin, 19, Value::Bool(dancing)) => m.extra.scratch[0] = i32::from(*dancing),
            // TamableAnimal.DATA_FLAGS_ID: in the sitting pose 1, tame 4.
            (Species::Wolf | Species::Cat | Species::Parrot, 18, Value::Byte(flags)) => {
                m.extra.sitting_pose = flags & 1 != 0;
                m.extra.tame = flags & 4 != 0;
            }
            // Cat.IS_LYING and RELAX_STATE_ONE.
            (Species::Cat, 21, Value::Bool(lying)) => m.extra.flag = *lying,
            (Species::Cat, 22, Value::Bool(relaxing)) => m.extra.relax_one = *relaxing,
            // Ocelot.DATA_TRUSTING (after AgeableMob's two).
            (Species::Ocelot, 18, Value::Bool(trusting)) => m.extra.trusting = *trusting,
            // Wolf.DATA_INTERESTED_ID: begging, which tilts the head.
            (Species::Wolf, 20, Value::Bool(begging)) => m.extra.flag = *begging,
            // Wolf.DATA_COLLAR_COLOR and Cat's.
            (Species::Wolf, 21, Value::Int(c)) | (Species::Cat, 23, Value::Int(c)) => {
                m.extra.color = (*c & 15) as u8;
            }
            // AbstractCubeMob.ID_SIZE: its size and box (refreshDimensions).
            (Species::Slime | Species::MagmaCube | Species::SulfurCube, 18, Value::Int(size)) => {
                m.extra.size = (*size).clamp(1, 127);
            }
            // SulfurCube.MAX_FUSE: onSyncedDataUpdated copies it into the fuse
            // the client ticks for swell / white flash (#4548); -1 is not primed.
            (Species::SulfurCube, 19, Value::Int(fuse)) => {
                m.extra.sulfur_fuse = sunrise_sim::mobs::sulfur_cube::Fuse::set(*fuse);
            }
            (Species::SulfurCube, 20, Value::Bool(from_bucket)) => {
                m.extra.scratch[3] = i32::from(*from_bucket);
            }
            // Phantom.ID_SIZE: `updatePhantomSizeInfo` grows its box and bite,
            // and `PhantomRenderer.scale` its model.
            (Species::Phantom, 16, Value::Int(size)) => phantom::set_size(m, *size),
            // Enderman.DATA_CREEPY (after DATA_CARRY_STATE): the open mouth
            // and raised head of an angry enderman.
            (Species::Enderman, 17, Value::Bool(creepy)) => {
                m.extra.flag = *creepy;
                if m.extra.flag2 {
                    stare_sound = sunrise_sim::mobs::enderman::stare_sound(m);
                }
            }
            (Species::Enderman, 18, Value::Bool(stared)) => m.extra.flag2 = *stared,
            // Warden.CLIENT_ANGER_LEVEL: the heartbeat's pace.
            (Species::Warden, 16, Value::Int(anger)) => {
                if let Some(st) = m.extra.warden.as_mut() {
                    st.client_anger = *anger;
                }
            }
            // Sniffer.DATA_STATE (after AgeableMob's two): the animation
            // follows it in `sync::client_tick`.
            (Species::Sniffer, 18, Value::Id(state)) => {
                sniffer::set_state(m, (*state).clamp(0, 6) as u8);
            }
            // Armadillo.ARMADILLO_STATE (after AgeableMob's two): the shell and
            // its animations follow it in `sync::client_tick`.
            (Species::Armadillo, 18, Value::Id(state)) => {
                armadillo::set_state(m, (*state).clamp(0, 3) as u8);
            }
            // Goat.DATA_HAS_LEFT_HORN and DATA_HAS_RIGHT_HORN: the model hides
            // a horn the goat lost.
            (Species::Goat, 19, Value::Bool(has)) => m.extra.lost_horns[0] = !*has,
            (Species::Goat, 20, Value::Bool(has)) => m.extra.lost_horns[1] = !*has,
            // Shulker.DATA_ATTACH_FACE_ID, DATA_PEEK_ID and DATA_COLOR_ID.
            (Species::Shulker, 16, Value::Id(face)) => m.extra.variant3 = (*face).clamp(0, 5) as u8,
            (Species::Shulker, 17, Value::Byte(peek)) => m.extra.counter = i32::from(*peek),
            (Species::Shulker, 18, Value::Byte(color)) => m.extra.color = *color as u8,
            // Camel.DASH and LAST_POSE_CHANGE_TICK (after AbstractHorse's
            // flags).
            (Species::Camel | Species::CamelHusk, 19, Value::Bool(dashing)) => {
                camel::set_dashing(m, *dashing);
            }
            (Species::Camel | Species::CamelHusk, 20, Value::Long(tick)) => {
                camel::set_synced_pose_tick(m, *tick);
            }
            // Vex.DATA_FLAGS_ID: charging in bit 0.
            (Species::Vex, 16, Value::Byte(flags)) => m.extra.flag = flags & 1 != 0,
            // GlowSquid.DATA_DARK_TICKS_REMAINING (after AgeableMob's two).
            (Species::GlowSquid, 18, Value::Int(ticks)) => {
                sunrise_sim::mobs::aquatic::set_dark_ticks(m, *ticks);
            }
            // Creaking.CAN_MOVE, IS_ACTIVE and IS_TEARING_DOWN.
            (Species::Creaking, 16, Value::Bool(can_move)) => m.extra.flag2 = !*can_move,
            (Species::Creaking, 17, Value::Bool(active)) => m.extra.flag = *active,
            (Species::Creaking, 18, Value::Bool(true)) => creaking::set_tearing_down(m),
            // Allay.DATA_DANCING and DATA_CAN_DUPLICATE (a cooldown running
            // is any non-zero `egg_time`).
            (Species::Allay, 16, Value::Bool(dancing)) => m.extra.flag = *dancing,
            (Species::Allay, 17, Value::Bool(can)) => m.extra.egg_time = i32::from(!*can),
            // Axolotl.DATA_PLAYING_DEAD (after DATA_VARIANT): the factors
            // ease in `sync::client_tick`.
            (Species::Axolotl, 19, Value::Bool(dead)) => m.extra.axolotl.playing_dead = *dead,
            // CopperGolem.COPPER_GOLEM_STATE (after DATA_WEATHER_STATE).
            (Species::CopperGolem, 17, Value::Id(state)) => {
                m.extra.variant3 = (*state).clamp(0, 4) as u8;
            }
            // AbstractHorse's flags take 18. Horse.DATA_ID_TYPE_VARIANT: the
            // colour in the low byte, the markings in the next.
            (Species::Horse, 19, Value::Int(v)) => {
                m.extra.variant = (v & 0xFF).clamp(0, 6) as u8;
                m.extra.variant2 = ((v >> 8) & 0xFF).clamp(0, 4) as u8;
            }
            // AbstractChestedHorse.DATA_ID_CHEST, then Llama.DATA_STRENGTH_ID
            // and DATA_VARIANT_ID.
            (
                Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama,
                19,
                Value::Bool(chested),
            ) => m.extra.chested = *chested,
            (Species::Llama | Species::TraderLlama, 20, Value::Int(strength)) => {
                m.extra.variant2 = (*strength).clamp(1, 5) as u8;
            }
            (Species::Llama | Species::TraderLlama, 21, Value::Int(variant)) => {
                m.extra.variant = (*variant).clamp(0, 3) as u8;
            }
            // Axolotl.DATA_VARIANT (lucy 0 to blue 4) and Parrot.DATA_VARIANT_ID
            // (after TamableAnimal's flags and owner).
            (Species::Axolotl, 18, Value::Int(v)) => m.extra.variant = (*v).clamp(0, 4) as u8,
            (Species::Parrot, 20, Value::Int(v)) => m.extra.variant = (*v).clamp(0, 4) as u8,
            // Rabbit.DATA_TYPE_ID: the killer bunny's 99 has no model here.
            (Species::Rabbit, 18, Value::Int(t)) if (0..=5).contains(t) => {
                m.extra.variant = *t as u8;
            }
            // Bee.DATA_FLAGS_ID and DATA_ANGER_END_TIME (after Animal's).
            (Species::Bee, 18, Value::Byte(flags)) => bee::set_synced(m, Some(*flags as u8), None),
            (Species::Bee, 19, Value::Long(end)) => bee::set_synced(m, None, Some(*end)),
            // AbstractFish.FROM_BUCKET takes 16: Salmon.DATA_TYPE (small 0
            // to large 2), Pufferfish.PUFF_STATE and
            // TropicalFish.DATA_ID_TYPE_VARIANT.
            (Species::Salmon, 17, Value::Int(v)) => m.extra.variant = (*v).clamp(0, 2) as u8,
            (Species::Pufferfish, 17, Value::Int(state)) => m.extra.size = (*state).clamp(0, 2),
            (Species::TropicalFish, 17, Value::Int(v)) => tropical_fish(m, *v),
            // Ghast.DATA_IS_CHARGING and WitherBoss.DATA_ID_INV.
            (Species::Ghast, 16, Value::Bool(charging)) => m.extra.flag = *charging,
            (Species::Wither, 19, Value::Int(ticks)) => m.extra.counter = (*ticks).max(0),
            // CopperGolem.DATA_WEATHER_STATE: the oxidation the texture shows.
            (Species::CopperGolem, 16, Value::Id(state)) => {
                m.extra.variant = (*state).clamp(0, 3) as u8;
            }
            // AbstractHorse.DATA_ID_FLAGS: eating 16 and standing 32 (the
            // graze and the rear), which `sync::client_tick` animates.
            (s, 18, Value::Byte(flags)) if equine::grazes(s) => equine::set_flags(m, *flags as u8),
            // Enderman.DATA_CARRY_STATE (0 for none): the block it holds.
            (Species::Enderman, 16, Value::Id(state)) => {
                m.extra.carried = (*state > 0).then_some(BlockStateId(*state as u32));
            }
            // Zombie.DATA_DROWNED_CONVERSION_ID (after DATA_BABY_ID and
            // DATA_SPECIAL_TYPE_ID): the conversion's shaking.
            (Species::Zombie | Species::Husk, 18, Value::Bool(converting)) => {
                m.extra.counter2 = i32::from(*converting);
            }
            // Skeleton.DATA_STRAY_CONVERSION_ID (the first entry after Mob's):
            // the freezing conversion's shaking.
            (Species::Skeleton, 16, Value::Bool(converting)) => {
                m.extra.counter2 = i32::from(*converting);
            }
            // ZombieVillager.DATA_CONVERTING_ID (before its villager data).
            (Species::ZombieVillager, 19, Value::Bool(converting)) => {
                zombie_villager::set_converting(m, *converting);
            }
            // AbstractVillager.DATA_UNHAPPY_COUNTER (after AgeableMob's two):
            // the head shake of `no`, which the server counts down.
            (Species::Villager | Species::WanderingTrader, 18, Value::Int(ticks)) => {
                m.extra.ints[0] = *ticks;
            }
            // Hoglin.DATA_IMMUNE_TO_ZOMBIFICATION (after AgeableMob's two) and
            // AbstractPiglin's: `sync`'s conversion shake reads it.
            (Species::Hoglin, 18, Value::Bool(immune))
            | (Species::Piglin | Species::PiglinBrute, 16, Value::Bool(immune)) => {
                m.extra.flag2 = *immune;
            }
            // Blaze.DATA_FLAGS_ID: charged 1, which makes it burn.
            (Species::Blaze, 16, Value::Byte(flags)) => m.extra.flag = flags & 1 != 0,
            // Turtle.HAS_EGG and LAYING_EGG (after AgeableMob's two).
            (Species::Turtle, 18, Value::Bool(has_egg)) => m.extra.flag = *has_egg,
            (Species::Turtle, 19, Value::Bool(laying)) => m.extra.flag2 = *laying,
            // WitherBoss.DATA_TARGET_B and _C (after DATA_TARGET_A): the
            // entities the side heads turn toward in `wither::client_tick`.
            (Species::Wither, 17, Value::Int(id)) => m.extra.ints[0] = *id,
            (Species::Wither, 18, Value::Int(id)) => m.extra.ints[1] = *id,
            // Raider.IS_CELEBRATING: shared by all six raider types.
            (species, 16, Value::Bool(celebrating)) if raider::is_raider(species) => {
                raider::state_mut(m).celebrating = *celebrating;
            }
            // Pillager.IS_CHARGING_CROSSBOW: the charge pose (Raider's
            // IS_CELEBRATING takes 16).
            (Species::Pillager, 17, Value::Bool(charging)) => m.extra.flag = *charging,
            // Witch.DATA_USING_ITEM: `isDrinkingPotion` (after Raider's IS_CELEBRATING).
            (Species::Witch, 17, Value::Bool(drinking)) => m.extra.flag = *drinking,
            // HappyGhast.STAYS_STILL (after AgeableMob's two and IS_LEASH_HOLDER):
            // `isOnStillTimeout` on a client, which keeps the ghast solid to
            // the player (`canBeCollidedWith`).
            // HappyGhast.IS_LEASH_HOLDER: RopesLayer draws the ropes while set.
            (Species::HappyGhast, 18, Value::Bool(holder)) => {
                m.extra.scratch[1] = i32::from(*holder)
            }
            (Species::HappyGhast, 19, Value::Bool(still)) => m.extra.counter = i32::from(*still),
            // SpellcasterIllager.DATA_SPELL_CASTING_ID: the casting pose.
            (Species::Evoker | Species::Illusioner, 17, Value::Byte(spell)) => {
                m.extra.variant2 = (*spell).max(0) as u8;
            }
            // Villager.DATA_VILLAGER_DATA and ZombieVillager's (after
            // Zombie's three): what VillagerProfessionLayer draws.
            (Species::Villager, 19, Value::Ids(data))
            | (Species::ZombieVillager, 20, Value::Ids(data)) => villager_data(m, data, builtin),
            // Mannequin.DATA_PROFILE (after Avatar's two): whose skin it
            // wears; update_remote_skins resolves it like a tab player's.
            (Species::Mannequin, 17, Value::Profile(profile)) => {
                m.profile = Some(profile.clone());
            }
            // Mannequin.DATA_IMMOVABLE and DATA_DESCRIPTION (an empty one is hidden).
            (Species::Mannequin, 18, Value::Bool(immovable)) => m.extra.immovable = *immovable,
            (Species::Mannequin, 19, Value::OptText(description)) => {
                m.extra.description =
                    sunrise_sim::mobs::MannequinDescription::from_component(description.as_ref());
            }
            _ => {}
        }
    }
    stare_sound
}

/// A tropical fish's packed variant: the pattern's id (the size, then the
/// index << 8), the base colour << 16 and the pattern colour << 24. Here the
/// pattern is 0 to 5 for the small ones and 6 to 11 for the large ones.
fn tropical_fish(m: &mut Mob, packed: i32) {
    let (size, index) = (packed & 0xFF, (packed >> 8) & 0xFF);
    // TropicalFish.Pattern.byId: an unknown id is a kob.
    m.extra.variant = if size <= 1 && index < 6 {
        (size * 6 + index) as u8
    } else {
        0
    };
    m.extra.variant2 = ((packed >> 16) & 0xFF) as u8 & 15;
    m.extra.variant3 = ((packed >> 24) & 0xFF) as u8 & 15;
}

/// The values of one SetEntityData packet that name an entry of a
/// synchronized registry (the variants of the cat, wolf, frog, pig, cow,
/// chicken and zombie nautilus) or count game time (a wolf's anger).
/// `game_time` is the client level's.
pub fn apply_registry_data(m: &mut Mob, values: &[(u8, i32, Value)], game_time: i64) {
    let species = m.species();
    for (index, _, value) in values {
        match (species, *index, value) {
            (Species::Cat, 20, Value::Id(id)) => {
                m.extra.cat_variant = Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Cat, 24, Value::Id(id)) => {
                m.extra.cat_sound_variant =
                    Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Wolf, 23, Value::Id(id)) => {
                m.extra.wolf_variant = Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Wolf, 24, Value::Id(id)) => {
                m.extra.wolf_sound_variant =
                    Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Frog, 18, Value::Id(id)) => {
                m.extra.frog_variant = Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Pig, 19, Value::Id(id)) => {
                m.extra.pig_variant = Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Pig, 20, Value::Id(id)) => {
                m.extra.pig_sound_variant =
                    Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Cow, 18, Value::Id(id)) => {
                m.extra.cow_variant = Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Cow, 19, Value::Id(id)) => {
                m.extra.cow_sound_variant =
                    Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Chicken, 18, Value::Id(id)) => {
                m.extra.chicken_variant =
                    Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::Chicken, 19, Value::Id(id)) => {
                m.extra.chicken_sound_variant =
                    Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            (Species::ZombieNautilus, 21, Value::Id(id)) => {
                m.extra.zombie_nautilus_variant =
                    Some(sunrise_sim::lan::data::RegistryRef::new(*id as u32));
            }
            // Wolf.DATA_ANGER_END_TIME: the angry texture while it lies ahead.
            (Species::Wolf, 22, Value::Long(end)) => {
                m.extra.anger = (end - game_time).clamp(0, i64::from(i32::MAX)) as i32;
            }
            _ => {}
        }
    }
}

/// What another player's pose draws from: gliding (Entity.isFallFlying, flag
/// 0x80 of the shared flags), the pose (swimming), Riptide's spin
/// (LivingEntity.isAutoSpinAttack, flag 4) and a loaded crossbow in the
/// hand (the held stack comes with SetEquipment).
fn player_poses(m: &mut Mob, values: &[(u8, i32, Value)]) {
    for (index, _, value) in values {
        match (*index, value) {
            (0, Value::Byte(flags)) => m.extra.fall_flying = flags & -128 != 0,
            (6, Value::Id(pose)) => m.extra.synced_pose = (*pose).clamp(0, 17) as u8,
            (8, Value::Byte(flags)) => m.extra.auto_spin_attack = flags & 4 != 0,
            // Player.DATA_SHOULDER_PARROT_LEFT and _RIGHT.
            (19, Value::Id(v)) => {
                m.extra.shoulder_parrots[0] = (*v > 0).then(|| (*v - 1).clamp(0, 4))
            }
            (20, Value::Id(v)) => {
                m.extra.shoulder_parrots[1] = (*v > 0).then(|| (*v - 1).clamp(0, 4))
            }
            _ => {}
        }
    }
}

/// The options of a synched particle list: an entity effect carries its ARGB
/// colour, the omens none.
fn swirls(list: &Particles, builtin: &Builtin) -> Vec<ParticleOptions> {
    let channel = |argb: i32, shift: u32| ((argb >> shift) & 255) as f32 / 255.0;
    list.iter()
        .filter_map(|p| {
            if let Some(ParticleData::Mod { name, options }) = &p.data {
                return sunrise_gameplay_common::game::lan_particle_data::decode_mod_particle(
                    name, options,
                );
            }
            let name = builtin.get("minecraft:particle_type")?.name(p.id)?;
            let ty = ParticleType::from_name(name)?;
            Some(match &p.data {
                Some(ParticleData::Color(argb)) => ParticleOptions::colored(
                    ty,
                    [
                        channel(*argb, 16),
                        channel(*argb, 8),
                        channel(*argb, 0),
                        channel(*argb, 24),
                    ],
                ),
                _ => ParticleOptions::simple(ty),
            })
        })
        .collect()
}

use sunrise_gameplay_common::game::villager_client::villager_data;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// SetEntityData for a mob of the server's.
    pub(in crate::game::client_side) fn remote_mob_data(
        &mut self,
        id: i32,
        values: &[(u8, i32, Value)],
    ) {
        let Some(r) = &self.client.remote else {
            return;
        };
        // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): any living entity
        // records its sleeping block pos; villagers also keep the legacy facing code.
        let sleeping_pos = values.iter().find_map(|v| match v {
            (14, _, Value::OptPos(bed)) => Some(*bed),
            _ => None,
        });
        let bed = sleeping_pos.map(|bed| bed.map_or(0, |b| self.common.bed_facing(b)));
        let sleep_at = sleeping_pos
            .flatten()
            .and_then(|head| self.sleeping_position(head));
        let professions = r.professions.clone();
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, id as u32)
        {
            let stare_sound = crate::game::client_side::villager_client_client::receive(
                m,
                values,
                &r.builtin,
                r.entity_id,
                &professions,
            );
            apply_registry_data(m, values, self.common.world.game_time);
            if let Some(pos) = sleeping_pos {
                m.extra.sleeping_pos = pos;
                if pos.is_some()
                    && let Some(at) = sleep_at
                {
                    // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): setPosToBed.
                    m.position = at;
                }
            }
            if let Some(code) = bed
                && m.species() == Species::Villager
            {
                m.extra.ints[1] = code;
            }
            if let Some(at) = stare_sound {
                self.play_audio_unshared("minecraft:entity.enderman.stare", Some(at), 2.5, 1.0);
            }
        }
    }
}

/// A mob as a client constructs it before its data arrives: the synched
/// defaults the server leaves out (`getNonDefaultValues`) that differ from
/// what `Mob::new` gives, such as a wolf's and a cat's red collar.
pub(in crate::game) fn client_defaults(m: &mut Mob) {
    m.air = m.max_air_supply();
    if matches!(m.species(), Species::Wolf | Species::Cat) {
        m.extra.color = tame::RED;
    }
    if m.species() == Species::Bat {
        // Bat's constructor sets resting only on the server; the data default is 0.
        bat::set_resting(m, false);
    }
    // A horse's jump strength until the server syncs its own: the type's.
    if equine::moved_by_rider(m.species()) {
        m.extra.vector[0] = m.kind.jump_strength;
    }
}

#[cfg(feature = "client")]
#[cfg(any(test, feature = "test-support"))]
fn fixture_mob_impl(kind: &str) -> Mob {
    use std::sync::Arc;
    use sunrise_sim::mobs::{Anim, Brain, Flags, MobType};
    let species = match kind {
        "minecraft:cow" => Species::Cow,
        "minecraft:frog" => Species::Frog,
        "minecraft:axolotl" => Species::Axolotl,
        "minecraft:dolphin" => Species::Dolphin,
        "minecraft:cod" => Species::Cod,
        "minecraft:pillager" => Species::Pillager,
        "minecraft:vindicator" => Species::Vindicator,
        "minecraft:evoker" => Species::Evoker,
        "minecraft:illusioner" => Species::Illusioner,
        "minecraft:witch" => Species::Witch,
        "minecraft:ravager" => Species::Ravager,
        "minecraft:zombie" => Species::Zombie,
        "minecraft:enderman" => Species::Enderman,
        "minecraft:piglin" => Species::Piglin,
        _ => panic!("unknown fixture species"),
    };
    // Metadata needs no AI or model. Avoid the resource loader, which also
    // initializes process-wide item tags used by unrelated mob tests.
    let kind = Arc::new(MobType {
        type_key: Default::default(),
        id: kind.into(),
        species,
        width: 0.6,
        height: 1.95,
        eye_height: 1.5,
        category: "monster".into(),
        max_health: 20.0,
        speed: 0.25,
        flying_speed: 0.0,
        attack_damage: 0.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor: 0.0,
        armor_toughness: 0.0,
        knockback_resistance: 0.0,
        jump_strength: 0.42,
        attributes: Arc::default(),
        item_stack_limits: Arc::default(),
        attribute_bounds: Default::default(),
        gravity: 0.08,
        safe_fall_distance: 3.0,
        fall_damage_multiplier: 1.0,
        water_efficiency: 0.0,
        water_slowdown: 0.8,
        air_speed: 0.02,
        baby_scale: 0.5,
        head_rot_speed: 10.0,
        max_head_x_rot: 40.0,
        max_head_y_rot: 75.0,
        ambient_interval: 120,
        flags: Flags::default(),
        brain: Brain::Passive,
        anim: Anim::Zombie,
        sounds: [""; 3],
        sound_overrides: Default::default(),
        food: Arc::default(),
        tempt: Arc::default(),
        passenger_attachments: Vec::new(),
    });
    Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0)
}

#[cfg(all(feature = "client", feature = "test-support", not(test)))]
pub mod tests {
    pub fn fixture_mob(kind: &str) -> super::Mob {
        super::fixture_mob_impl(kind)
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub mod tests {
    use super::*;
    use sunrise_protocol::entity_data::serializer as s;

    pub fn fixture_mob(kind: &str) -> Mob {
        super::fixture_mob_impl(kind)
    }

    #[test]
    fn enderman_stare_metadata_requests_one_local_sound_with_cooldown() {
        let mut mob = fixture_mob("minecraft:enderman");
        let builtin = Builtin::default();
        let apply =
            |mob: &mut Mob, values: &[(u8, i32, Value)]| super::apply(mob, values, &builtin, 99);
        assert_eq!(
            apply(&mut mob, &[(18, s::BOOLEAN, Value::Bool(true))]),
            None
        );
        assert_eq!(
            apply(&mut mob, &[(17, s::BOOLEAN, Value::Bool(true))]),
            Some([0.0, 1.5, 0.0])
        );
        mob.age = 399;
        assert_eq!(
            apply(&mut mob, &[(17, s::BOOLEAN, Value::Bool(false))]),
            None
        );
        mob.age = 400;
        assert_eq!(
            apply(&mut mob, &[(17, s::BOOLEAN, Value::Bool(true))]),
            Some([0.0, 1.5, 0.0])
        );
        mob.age = 800;
        mob.silent = true;
        assert_eq!(
            apply(&mut mob, &[(17, s::BOOLEAN, Value::Bool(false))]),
            None
        );
        mob.silent = false;
        assert_eq!(
            apply(&mut mob, &[(17, s::BOOLEAN, Value::Bool(true))]),
            None
        );
        mob.age = 1200;
        // Entries are assigned in packet order: a clear of creepy still sees
        // the previous stared-at flag before the following clear of index 18.
        assert!(
            apply(
                &mut mob,
                &[
                    (17, s::BOOLEAN, Value::Bool(false)),
                    (18, s::BOOLEAN, Value::Bool(false)),
                ]
            )
            .is_some()
        );
        assert_eq!(
            apply(&mut mob, &[(17, s::BOOLEAN, Value::Bool(true))]),
            None
        );
    }

    #[test]
    fn air_and_frozen_metadata_reach_the_client() {
        for kind in ["cow", "cod", "dolphin", "axolotl"] {
            let mut mob = fixture_mob(&format!("minecraft:{kind}"));
            super::client_defaults(&mut mob);
            let max = match kind {
                "axolotl" => 6000,
                "dolphin" => 4800,
                _ => 300,
            };
            assert_eq!(mob.air, max, "initial air: {kind}");
            for (air, frozen) in [(100, -3), (-20, 140), (max, 0)] {
                let _ = super::apply(
                    &mut mob,
                    &[
                        (1, s::INT, Value::Int(air)),
                        (7, s::INT, Value::Int(frozen)),
                    ],
                    &Builtin::default(),
                    99,
                );
                assert_eq!((mob.air, mob.ticks_frozen), (air, frozen), "{kind}");
            }
        }
    }

    fn air_trace(kind: &str, sequence: &[(i32, i32)]) -> Vec<String> {
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        use sunrise_sim::lan::entity::{Entities, EntityState, Riding, Tracker, Viewer};
        let name = format!("minecraft:{kind}");
        let mut server = fixture_mob(&name);
        let mut client = fixture_mob(&name);
        super::client_defaults(&mut client);
        let mut level = Entities::default();
        let mut tracker = Tracker::default();
        let chunks = [(0, 0)].into_iter().collect();
        let mut state = EntityState {
            id: 1,
            uuid: 1,
            kind: name,
            needs_sync: true,
            ..Default::default()
        };
        let mut trace = Vec::new();
        for &(air, frozen) in sequence {
            server.air = air;
            server.ticks_frozen = frozen;
            state.meta.clear();
            state.defaults.clear();
            sunrise_gameplay_common::game::lan_mob_metadata::mob_freeze_meta(
                &server,
                &mut state.meta,
                &mut state.defaults,
            );
            let mut packets = Vec::new();
            level.tick(
                &[&state],
                &Riding::default(),
                &mut [Viewer {
                    entity_id: 99,
                    spectator: false,
                    permission: 0,
                    sunrise: true,
                    general_entity: true,
                    attachment_sender: None,
                    moved: true,
                    position: [0.0; 3],
                    view_distance: 10,
                    chunks: &chunks,
                    sources: None,
                    far: &[],
                    tracker: &mut tracker,
                    out: &mut packets,
                }],
                None,
                |_| Some(100),
            );
            let mut sent = Vec::new();
            for body in packets {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                if let Ok(Clientbound::SetEntityData(data)) = Clientbound::decode(id, reader.rest())
                {
                    for (index, serializer, value) in &data.values {
                        if matches!(*index, 1 | 7) {
                            assert_eq!(*serializer, s::INT);
                            let Value::Int(value) = value else {
                                panic!("integer metadata");
                            };
                            sent.push((*index, *value));
                        }
                    }
                    let _ = super::apply(&mut client, &data.values, &Builtin::default(), 99);
                }
            }
            sent.sort_unstable_by_key(|v| v.0);
            let sent = if sent.is_empty() {
                "-".into()
            } else {
                sent.iter()
                    .map(|(index, value)| format!("{index}={value}"))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            trace.push(format!("{sent}:{},{}", client.air, client.ticks_frozen));
        }
        trace
    }

    #[test]
    fn air_and_frozen_cross_the_tracker_and_wire() {
        for (kind, max) in [
            ("cow", 300),
            ("cod", 300),
            ("dolphin", 4800),
            ("axolotl", 6000),
        ] {
            let sequence = [(max, 0), (100, -3), (100, -3), (max, 0), (max, 0)];
            assert_eq!(
                air_trace(kind, &sequence),
                vec![
                    format!("-:{max},0"),
                    "1=100,7=-3:100,-3".into(),
                    "-:100,-3".into(),
                    format!("1={max},7=0:{max},0"),
                    format!("-:{max},0")
                ],
                "{kind}"
            );
            assert_eq!(
                air_trace(kind, &[(100, -3)]),
                vec!["1=100,7=-3:100,-3"],
                "joining while changed: {kind}"
            );
        }
    }

    #[test]
    #[ignore = "run by tools/test_mob_air_parity.py"]
    fn mob_air_parity_fixture() {
        run_mob_air_parity_fixture();
    }

    pub(in crate::game::client_side) fn run_mob_air_parity_fixture() {
        use std::io::Write;
        let input =
            std::fs::read_to_string(std::env::var("SUNRISE_MOB_AIR_INPUT").unwrap()).unwrap();
        let mut output =
            std::fs::File::create(std::env::var("SUNRISE_MOB_AIR_OUTPUT").unwrap()).unwrap();
        for line in input.lines() {
            let mut fields = line.split_whitespace();
            let kind = fields.next().unwrap();
            let sequence: Vec<_> = fields
                .map(|pair| {
                    let (air, frozen) = pair.split_once(',').unwrap();
                    (air.parse().unwrap(), frozen.parse().unwrap())
                })
                .collect();
            writeln!(output, "{}", air_trace(kind, &sequence).join(" ")).unwrap();
        }
    }

    #[test]
    fn raider_celebration_follows_metadata_and_clears() {
        use sunrise_sim::mobs::raider;
        for kind in [
            "pillager",
            "vindicator",
            "evoker",
            "illusioner",
            "witch",
            "ravager",
        ] {
            let mut m = fixture_mob(&format!("minecraft:{kind}"));
            for on in [true, false, true, false] {
                let _ = super::apply(
                    &mut m,
                    &[(16, s::BOOLEAN, Value::Bool(on))],
                    &Builtin::default(),
                    99,
                );
                assert_eq!(
                    raider::state(&m).is_some_and(|state| state.celebrating),
                    on,
                    "{kind}"
                );
            }
        }
        for kind in ["zombie", "cow"] {
            let mut m = fixture_mob(&format!("minecraft:{kind}"));
            let _ = super::apply(
                &mut m,
                &[(16, s::BOOLEAN, Value::Bool(true))],
                &Builtin::default(),
                99,
            );
            assert!(raider::state(&m).is_none(), "{kind} is not a raider");
        }
    }

    fn celebration_trace(kind: &str, sequence: &[bool]) -> Vec<(Vec<bool>, bool)> {
        use sunrise_sim::mobs::raider;
        raider_flag_trace(
            kind,
            16,
            |m, on| raider::state_mut(m).celebrating = on,
            |m| raider::state(m).is_some_and(|state| state.celebrating),
            sequence,
        )
    }

    /// A raider's boolean at `index`, set on the host by `set` for each step of `sequence`,
    /// sent by `mob_raider_meta` through the tracker and the wire to a client copy: per step,
    /// what went out at `index` and what the client then reads (`read`).
    fn raider_flag_trace(
        kind: &str,
        index: u8,
        set: impl Fn(&mut Mob, bool),
        read: impl Fn(&Mob) -> bool,
        sequence: &[bool],
    ) -> Vec<(Vec<bool>, bool)> {
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        use sunrise_sim::lan::entity::{Entities, EntityState, Riding, Tracker, Viewer};
        let name = format!("minecraft:{kind}");
        let mut server = fixture_mob(&name);
        let mut client = fixture_mob(&name);
        let mut level = Entities::default();
        let mut tracker = Tracker::default();
        let chunks = [(0, 0)].into_iter().collect();
        let mut state = EntityState {
            id: 1,
            uuid: 1,
            kind: name,
            needs_sync: true,
            ..Default::default()
        };
        let mut trace = Vec::new();
        for &on in sequence {
            set(&mut server, on);
            state.meta.clear();
            state.defaults.clear();
            sunrise_gameplay_common::game::lan_mob_metadata::mob_raider_meta(
                &server,
                &mut state.meta,
                &mut state.defaults,
            );
            let mut packets = Vec::new();
            level.tick(
                &[&state],
                &Riding::default(),
                &mut [Viewer {
                    entity_id: 99,
                    spectator: false,
                    permission: 0,
                    sunrise: true,
                    general_entity: true,
                    attachment_sender: None,
                    moved: true,
                    position: [0.0; 3],
                    view_distance: 10,
                    chunks: &chunks,
                    sources: None,
                    far: &[],
                    tracker: &mut tracker,
                    out: &mut packets,
                }],
                None,
                |_| Some(100),
            );
            let mut sent = Vec::new();
            for body in packets {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                if let Ok(Clientbound::SetEntityData(data)) = Clientbound::decode(id, reader.rest())
                {
                    for (at, serializer, value) in &data.values {
                        if *at == index {
                            assert_eq!(*serializer, s::BOOLEAN);
                            let Value::Bool(on) = value else {
                                panic!("boolean metadata");
                            };
                            sent.push(*on);
                        }
                    }
                    let _ = super::apply(&mut client, &data.values, &Builtin::default(), 99);
                }
            }
            trace.push((sent, read(&client)));
        }
        trace
    }

    /// Witch.DATA_USING_ITEM (17): her drink goes out on her own flag, the end
    /// of it too, and nothing repeats.
    #[test]
    fn a_witchs_drink_crosses_the_tracker_on_her_own_flag() {
        let drink = |m: &mut Mob, on: bool| {
            m.extra.flag = on;
            m.extra.using_item = if on { 32 } else { 0 };
        };
        let sequence = [false, true, true, false, true, false];
        let expected = vec![
            (vec![], false),
            (vec![true], true),
            (vec![], true),
            (vec![false], false),
            (vec![true], true),
            (vec![false], false),
        ];
        let trace = raider_flag_trace("witch", 17, drink, |m| m.extra.flag, &sequence);
        assert_eq!(trace, expected);

        // A pillager's 17 is its crossbow charge, still.
        let mut pillager = fixture_mob("minecraft:pillager");
        let _ = super::apply(
            &mut pillager,
            &[(17, s::BOOLEAN, Value::Bool(true))],
            &Builtin::default(),
            99,
        );
        assert!(pillager.extra.flag);
    }

    #[test]
    fn celebration_crosses_the_tracker_and_wire_without_sticking_or_repeating() {
        let sequence = [false, true, true, false, false, true, false];
        let expected = vec![
            (vec![], false),
            (vec![true], true),
            (vec![], true),
            (vec![false], false),
            (vec![], false),
            (vec![true], true),
            (vec![false], false),
        ];
        for kind in [
            "pillager",
            "vindicator",
            "evoker",
            "illusioner",
            "witch",
            "ravager",
        ] {
            assert_eq!(celebration_trace(kind, &sequence), expected, "{kind}");
            assert_eq!(
                celebration_trace(kind, &[true]),
                vec![(vec![true], true)],
                "joining mid-celebration: {kind}"
            );
        }
    }

    #[test]
    #[ignore = "run by tools/test_raider_celebration_parity.py"]
    fn raider_celebration_parity_fixture() {
        run_raider_celebration_parity_fixture();
    }

    pub(in crate::game::client_side) fn run_raider_celebration_parity_fixture() {
        use std::io::Write;
        let input =
            std::fs::read_to_string(std::env::var("SUNRISE_RAIDER_SYNC_INPUT").unwrap()).unwrap();
        let mut output =
            std::fs::File::create(std::env::var("SUNRISE_RAIDER_SYNC_OUTPUT").unwrap()).unwrap();
        for line in input.lines() {
            let mut fields = line.split_whitespace();
            let kind = fields.next().unwrap();
            let sequence: Vec<_> = fields
                .map(|value| match value {
                    "0" => false,
                    "1" => true,
                    _ => panic!("invalid celebration flag"),
                })
                .collect();
            let trace = celebration_trace(kind, &sequence);
            let mut parts = Vec::new();
            for (sent, state) in trace {
                let sent = if sent.is_empty() {
                    "-".into()
                } else {
                    sent.into_iter()
                        .map(|on| i32::from(on).to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                };
                parts.push(format!("{sent}:{}", i32::from(state)));
            }
            writeln!(output, "{}", parts.join(" ")).unwrap();
        }
    }

    /// A mob of the prepared resources' type `kind`, as a client spawns it
    /// (None without the resources).
    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION)
    }

    /// The prepared resources' built-in registries (empty without them).
    fn builtin() -> Builtin {
        Builtin::load(&root().join("registries.json")).unwrap_or_default()
    }

    fn apply(m: &mut Mob, values: &[(u8, i32, Value)]) {
        let _ = super::apply(m, values, &builtin(), 99);
    }

    fn mob(kind: &str) -> Option<Mob> {
        let types = sunrise_sim::mobs::MobTypes::load(&root()).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        let mut m = Mob::new(1, t, sunrise_core::geometry::Vec3::ZERO, 0.0);
        client_defaults(&mut m);
        Some(m)
    }

    #[test]
    fn received_invisible_illusioner_hides_body() {
        let Some(mut m) = mob("minecraft:illusioner") else {
            return;
        };
        apply(&mut m, &[(0, s::BYTE, Value::Byte(32))]);
        assert!(
            m.is_invisible(),
            "a received shared flag must hide the illusioner's body"
        );
        apply(&mut m, &[(0, s::BYTE, Value::Byte(0))]);
        assert!(!m.is_invisible());
    }

    /// #2920: a server's horse is moved by its rider's client, which jumps
    /// it by the type's jump strength (`createBaseHorseAttributes`,
    /// `createBaseChestedHorseAttributes`) until the server syncs its own.
    #[test]
    fn a_servers_horse_jumps_its_types_strength() {
        for (kind, strength) in [("horse", 0.7), ("donkey", 0.5), ("mule", 0.5)] {
            let Some(m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert!(
                (m.extra.vector[0] - strength).abs() < 1e-6,
                "{kind}: {}",
                m.extra.vector[0]
            );
        }
    }

    #[test]
    fn guardian_charge_follows_target_updates_and_caps_at_its_duration() {
        use sunrise_sim::mobs::guardian::{beam, client_tick};
        for (kind, duration) in [("guardian", 80), ("elder_guardian", 60)] {
            let Some(mut m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert!(beam(&m, 0.0).is_none());
            apply(
                &mut m,
                &[
                    (16, s::BOOLEAN, Value::Bool(true)),
                    (17, s::INT, Value::Int(99)),
                ],
            );
            assert!(m.extra.flag2);
            assert_eq!(m.ai.target, Some(Who::Player));
            assert_eq!(beam(&m, 0.0), Some((0.0, 0.0)));
            for _ in 0..duration + 10 {
                client_tick(&mut m);
            }
            assert_eq!(beam(&m, 0.0), Some((duration as f32, 1.0)));
            // Another player or a mob retains its network entity id.
            apply(&mut m, &[(17, s::INT, Value::Int(42))]);
            assert_eq!(m.ai.target, Some(Who::Mob(42)));
            assert_eq!(m.extra.counter2, 0);
            client_tick(&mut m);
            assert_eq!(m.extra.counter2, 1);
            apply(
                &mut m,
                &[
                    (16, s::BOOLEAN, Value::Bool(false)),
                    (17, s::INT, Value::Int(0)),
                ],
            );
            client_tick(&mut m);
            assert!(!m.extra.flag2);
            assert_eq!(m.ai.target, None);
            assert_eq!(m.extra.counter2, 0);
            assert!(beam(&m, 0.0).is_none());
        }
    }

    #[test]
    fn a_mooshroom_takes_its_type_after_the_age_lock() {
        let Some(mut m) = mob("minecraft:mooshroom") else {
            return;
        };
        // AgeableMob.AGE_LOCKED (17) is not the colour.
        apply(&mut m, &[(17, s::BOOLEAN, Value::Bool(true))]);
        assert_eq!(m.extra.variant, 0);
        // MushroomCow.DATA_TYPE (18): what MushroomCow.thunderHit changes.
        apply(&mut m, &[(18, s::INT, Value::Int(1))]);
        assert_eq!(m.extra.variant, 1);
        apply(&mut m, &[(18, s::INT, Value::Int(0))]);
        assert_eq!(m.extra.variant, 0);
    }

    #[test]
    fn a_sheep_takes_its_wool_and_a_calf_its_size() {
        let Some(mut m) = mob("minecraft:sheep") else {
            return;
        };
        apply(&mut m, &[(18, s::BYTE, Value::Byte(14 | 16))]);
        assert_eq!((m.extra.color, m.extra.sheared), (14, true));
        apply(&mut m, &[(18, s::BYTE, Value::Byte(0))]);
        assert_eq!((m.extra.color, m.extra.sheared), (0, false));
        let mut calf = mob("minecraft:cow").unwrap();
        apply(&mut calf, &[(16, s::BOOLEAN, Value::Bool(true))]);
        assert!(calf.is_baby());
        apply(&mut calf, &[(16, s::BOOLEAN, Value::Bool(false))]);
        assert!(!calf.is_baby());
    }

    /// #6133: a server's baby happy ghast (a hatched dried ghast) and baby
    /// nautilus come with AgeableMob.DATA_BABY_ID (16) like any other animal;
    /// a parrot never is one (`Parrot.canBeABaby`).
    #[test]
    fn a_happy_ghast_and_a_nautilus_take_their_babyhood() {
        use std::{fs, sync::Arc};
        use sunrise_sim::mobs::MobTypes;

        let root = std::env::temp_dir().join(format!("sunrise-baby-flag-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("entity-types.json"),
            r#"{"minecraft:happy_ghast": {}, "minecraft:nautilus": {}, "minecraft:parrot": {}}"#,
        )
        .unwrap();
        fs::write(
            root.join("entity-models.json"),
            r#"{"minecraft:happy_ghast#main": {}, "minecraft:nautilus#main": {},
                "minecraft:parrot#main": {}}"#,
        )
        .unwrap();
        fs::write(
            root.join("attributes.json"),
            format!(
                r#"{{"data_version": {}, "attributes": [], "entities": {{}}}}"#,
                sunrise_core::version::DATA_VERSION
            ),
        )
        .unwrap();
        let mut types = MobTypes::load(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
        for kind in ["minecraft:happy_ghast", "minecraft:nautilus"] {
            let t = Arc::new(types.types.remove(kind).unwrap());
            let mut m = Mob::new(1, t, sunrise_core::geometry::Vec3::ZERO, 0.0);
            client_defaults(&mut m);
            assert!(!m.is_baby(), "{kind}");
            apply(&mut m, &[(16, s::BOOLEAN, Value::Bool(true))]);
            assert!(m.is_baby(), "{kind}");
            apply(&mut m, &[(16, s::BOOLEAN, Value::Bool(false))]);
            assert!(!m.is_baby(), "{kind}");
        }
        let t = Arc::new(types.types.remove("minecraft:parrot").unwrap());
        let mut parrot = Mob::new(1, t, sunrise_core::geometry::Vec3::ZERO, 0.0);
        client_defaults(&mut parrot);
        apply(&mut parrot, &[(16, s::BOOLEAN, Value::Bool(true))]);
        assert!(!parrot.is_baby());
    }

    #[test]
    fn cube_mobs_receive_size_from_metadata() {
        for kind in ["slime", "magma_cube", "sulfur_cube"] {
            let Some(mut m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert_eq!(m.extra.size, 1);
            apply(&mut m, &[(18, s::INT, Value::Int(2))]);
            assert_eq!(m.extra.size, 2);
            if kind == "sulfur_cube" {
                assert!(!m.is_baby());
            }
        }
    }

    #[test]
    fn bat_flags_select_continuous_client_clocks_without_changing_motion() {
        use serde_json::json;
        use std::{collections::HashMap, fs, sync::Arc};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::{MobTypes, anim, keyframes::Animation};

        // Original minimal resources keep this regression runnable without vanilla data.
        let root =
            std::env::temp_dir().join(format!("sunrise-bat-animation-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("entity-types.json"), r#"{"minecraft:bat": {}}"#).unwrap();
        fs::write(
            root.join("entity-models.json"),
            r#"{"minecraft:bat#main": {"children": {"right_wing": {}, "head": {}}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("attributes.json"),
            format!(
                r#"{{"data_version": {}, "attributes": [], "entities": {{}}}}"#,
                sunrise_core::version::DATA_VERSION
            ),
        )
        .unwrap();
        let mut types = MobTypes::load(&root).unwrap();
        let models = sunrise_client::mobs_render::ClientMobModels::load(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
        let position = Vec3::new(1.0, 2.25, 3.0);
        let velocity = Vec3::new(0.2, 0.3, 0.4);
        let mut m = Mob::new(
            1,
            Arc::new(types.types.remove("minecraft:bat").unwrap()),
            position,
            0.0,
        );
        assert!(bat::resting(&m));
        client_defaults(&mut m);
        assert!(!bat::resting(&m));
        m.velocity = velocity;
        m.age = 1;
        bat::setup_animation_states(&mut m);
        m.age = 4;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(2))]);
        bat::setup_animation_states(&mut m);
        assert_eq!(m.anim.states[0].started, Some(1.0));

        let curve = Animation::parse(&json!({"length": 1, "looping": true,
            "bones": {"right_wing": [{"target": "rotation", "keyframes":
                [[0,[0,0,0],[0,0,0],"linear"],[1,[0,1,0],[0,1,0],"linear"]]}]}}));
        let animations = HashMap::from([("BatAnimation.BAT_FLYING".into(), curve)]);
        let wing = |partial| {
            let mut poses = anim::Poses::new(models.model(&m.kind.id).unwrap());
            anim::setup(&m, &anim::state(&m, partial), &mut poses, &animations);
            poses.get("right_wing")[4]
        };
        assert!(wing(0.5) > wing(0.0));

        m.age = 5;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(3))]);
        bat::setup_animation_states(&mut m);
        assert!(bat::resting(&m));
        assert_eq!(m.anim.states[0].started, None);
        assert_eq!(m.anim.states[1].started, Some(5.0));
        m.age = 7;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(0))]);
        bat::setup_animation_states(&mut m);
        assert_eq!(m.anim.states[0].started, Some(7.0));
        assert_eq!(m.anim.states[1].started, None);
        assert_eq!((m.position, m.velocity), (position, velocity));
    }

    #[test]
    fn a_wolf_takes_its_owner_flags_and_collar() {
        let Some(mut m) = mob("minecraft:wolf") else {
            return;
        };
        assert_eq!(m.extra.color, tame::RED);
        apply(
            &mut m,
            &[
                (18, s::BYTE, Value::Byte(4 | 1)),
                (21, s::INT, Value::Int(11)),
            ],
        );
        assert!(m.extra.tame && m.extra.sitting_pose);
        assert_eq!(m.extra.color, 11);
    }

    /// Wolf.DATA_INTERESTED_ID: the begging flag reaches the client, whose
    /// own `Wolf.tick` eases `interestedAngle` toward it (0.4 of the way a
    /// tick), and `getHeadRollAngle` tilts the head by it.
    #[test]
    fn a_wolf_tilts_its_head_while_the_server_says_it_begs() {
        use sunrise_sim::mobs::sync::client_tick;
        let Some(mut m) = mob("minecraft:wolf") else {
            return;
        };
        apply(&mut m, &[(20, s::BOOLEAN, Value::Bool(true))]);
        client_tick(&mut m, 0, &mut Vec::new());
        assert!((m.anim.v[tame::INTEREST] - 0.4).abs() < 1e-6);
        assert_eq!(m.anim.vo[tame::INTEREST], 0.0);
        client_tick(&mut m, 0, &mut Vec::new());
        assert!((m.anim.v[tame::INTEREST] - 0.64).abs() < 1e-6);
        assert!((m.anim.lerp(tame::INTEREST, 0.5) - 0.52).abs() < 1e-6);
        for _ in 0..30 {
            client_tick(&mut m, 0, &mut Vec::new());
        }
        assert!(m.anim.v[tame::INTEREST] > 0.999);
        apply(&mut m, &[(20, s::BOOLEAN, Value::Bool(false))]);
        client_tick(&mut m, 0, &mut Vec::new());
        assert!(m.anim.v[tame::INTEREST] < 0.61);
        for _ in 0..30 {
            client_tick(&mut m, 0, &mut Vec::new());
        }
        assert!(m.anim.v[tame::INTEREST] < 0.001);
    }

    #[test]
    fn fire_health_and_size_follow_the_server() {
        let Some(mut m) = mob("minecraft:slime") else {
            return;
        };
        apply(
            &mut m,
            &[
                (0, s::BYTE, Value::Byte(1)),
                (9, s::FLOAT, Value::Float(12.5)),
                (18, s::INT, Value::Int(4)),
            ],
        );
        assert_eq!((m.fire_ticks, m.health, m.extra.size), (1, 12.5, 4));
        apply(&mut m, &[(0, s::BYTE, Value::Byte(0))]);
        assert_eq!(m.fire_ticks, 0);
    }

    #[test]
    fn mob_flags_turn_aggression_on_and_off() {
        let Some(mut m) = mob("minecraft:vindicator") else {
            return;
        };
        apply(&mut m, &[(15, s::BYTE, Value::Byte(4 | 2))]);
        assert!(m.ai.aggressive && m.extra.left_handed && !m.no_ai);
        assert!(sunrise_sim::mobs::illager::shows_item(&m));
        apply(&mut m, &[(15, s::BYTE, Value::Byte(0))]);
        assert!(!m.ai.aggressive && !m.extra.left_handed);
        assert!(!sunrise_sim::mobs::illager::shows_item(&m));
    }

    #[test]
    fn a_villager_takes_its_type_profession_and_level() {
        let Some(mut m) = mob("minecraft:villager") else {
            return;
        };
        let registries = builtin();
        let id = |registry: &str, name: &str| registries.get(registry)?.id(name);
        let (Some(desert), Some(farmer), Some(librarian)) = (
            id("minecraft:villager_type", "minecraft:desert"),
            id("minecraft:villager_profession", "minecraft:farmer"),
            id("minecraft:villager_profession", "minecraft:librarian"),
        ) else {
            return;
        };
        let name = |m: &Mob| {
            (
                VILLAGER_TYPES[m.extra.variant as usize],
                sunrise_sim::mobs::villager::profession_table().entries()
                    [m.extra.profession.index().unwrap_or(0) as usize]
                    .id
                    .as_str(),
                m.extra.villager_level,
            )
        };
        apply(
            &mut m,
            &[(19, s::VILLAGER_DATA, Value::Ids(vec![desert, farmer, 2]))],
        );
        assert_eq!(name(&m), ("desert", "farmer", 2));
        // A job taken later: only the profession and the level change.
        apply(
            &mut m,
            &[(19, s::VILLAGER_DATA, Value::Ids(vec![desert, librarian, 5]))],
        );
        assert_eq!(name(&m), ("desert", "librarian", 5));
        apply(
            &mut m,
            &[(
                19,
                s::VILLAGER_DATA,
                Value::Ids(vec![desert, librarian, 123456]),
            )],
        );
        assert_eq!(name(&m), ("desert", "librarian", 123456));
        // A zombie villager's data comes after Zombie's three entries.
        let mut z = mob("minecraft:zombie_villager").unwrap();
        apply(
            &mut z,
            &[(20, s::VILLAGER_DATA, Value::Ids(vec![desert, farmer, 3]))],
        );
        assert_eq!(name(&z), ("desert", "farmer", 3));
    }

    #[test]
    fn the_mob_flags_drive_the_aggressive_poses() {
        let Some(mut m) = mob("minecraft:skeleton") else {
            return;
        };
        apply(&mut m, &[(15, s::BYTE, Value::Byte(2 | 4))]);
        assert!(m.ai.aggressive && m.extra.left_handed && !m.no_ai);
        apply(&mut m, &[(15, s::BYTE, Value::Byte(0))]);
        assert!(!m.ai.aggressive && !m.extra.left_handed);
    }

    #[test]
    fn the_living_flags_start_and_stop_a_mob_using_an_item() {
        let Some(mut m) = mob("minecraft:skeleton") else {
            return;
        };
        apply(&mut m, &[(8, s::BYTE, Value::Byte(1))]);
        assert_eq!(m.extra.using_item, 1);
        assert!(!m.extra.using_off_hand);
        // Using in the off hand: bit 0 for the use, bit 1 for the hand.
        apply(&mut m, &[(8, s::BYTE, Value::Byte(3))]);
        assert_eq!(m.extra.using_item, 1);
        assert!(m.extra.using_off_hand);
        // LivingEntity.stopUsingItem clears bit 0 only, so the stop of an
        // off-hand use arrives as 2.
        apply(&mut m, &[(8, s::BYTE, Value::Byte(2))]);
        assert_eq!(m.extra.using_item, 0);
        assert!(m.extra.using_off_hand);
        apply(&mut m, &[(8, s::BYTE, Value::Byte(0))]);
        assert_eq!(m.extra.using_item, 0);
    }

    /// LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID of another
    /// player, which ArrowLayer and BeeStingerLayer draw, and of a mob.
    #[test]
    fn stuck_arrows_and_stingers_are_kept() {
        for kind in ["minecraft:player", "minecraft:zombie"] {
            let Some(mut m) = mob(kind) else {
                return;
            };
            apply(&mut m, &[(12, s::INT, Value::Int(4))]);
            apply(&mut m, &[(13, s::INT, Value::Int(2))]);
            assert_eq!((m.extra.stuck.arrows, m.extra.stuck.stingers), (4, 2));
            apply(&mut m, &[(12, s::INT, Value::Int(0))]);
            assert_eq!((m.extra.stuck.arrows, m.extra.stuck.stingers), (0, 2));
        }
    }

    #[test]
    fn a_pillager_charges_and_a_spellcaster_casts() {
        let Some(mut p) = mob("minecraft:pillager") else {
            return;
        };
        apply(&mut p, &[(17, s::BOOLEAN, Value::Bool(true))]);
        assert!(p.extra.flag);
        let mut e = mob("minecraft:evoker").unwrap();
        apply(&mut e, &[(17, s::BYTE, Value::Byte(2))]);
        assert_eq!(e.extra.variant2, 2);
    }

    #[test]
    fn a_mannequin_exists_and_takes_its_profile() {
        let Some(mut m) = mob("minecraft:mannequin") else {
            return;
        };
        assert_eq!(&*m.kind.id, "minecraft:mannequin");
        let profile = sunrise_protocol::entity_data::Profile {
            name: Some("Gommunity".into()),
            uuid: Some(1),
            ..Default::default()
        };
        apply(
            &mut m,
            &[(17, s::RESOLVABLE_PROFILE, Value::Profile(profile.clone()))],
        );
        assert_eq!(m.profile, Some(profile));
    }

    /// `Ocelot.DATA_TRUSTING` (after AgeableMob's two).
    #[test]
    fn an_ocelot_takes_its_trusting_flag() {
        let Some(mut m) = mob("minecraft:ocelot") else {
            return;
        };
        assert!(!m.extra.trusting);
        apply(&mut m, &[(18, s::BOOLEAN, Value::Bool(true))]);
        assert!(m.extra.trusting);
        apply(&mut m, &[(18, s::BOOLEAN, Value::Bool(false))]);
        assert!(!m.extra.trusting);
    }

    /// `Mannequin.DATA_IMMOVABLE` and `DATA_DESCRIPTION`: the default component is the default
    /// description, an empty one hides it, any other is the server's own.
    #[test]
    fn a_mannequin_takes_its_immovable_flag_and_description() {
        use sunrise_sim::mobs::MannequinDescription as D;
        let Some(mut m) = mob("minecraft:mannequin") else {
            return;
        };
        assert_eq!(
            (m.extra.immovable, m.extra.description.clone()),
            (false, D::Default)
        );
        let own = sunrise_protocol::text::literal("Guard");
        apply(
            &mut m,
            &[
                (18, s::BOOLEAN, Value::Bool(true)),
                (19, s::OPTIONAL_COMPONENT, Value::OptText(Some(own.clone()))),
            ],
        );
        assert_eq!(
            (m.extra.immovable, m.extra.description.clone()),
            (true, D::Custom(own))
        );
        apply(&mut m, &[(19, s::OPTIONAL_COMPONENT, Value::OptText(None))]);
        assert_eq!(m.extra.description, D::Hidden);
        let default = D::Default.component();
        apply(
            &mut m,
            &[(19, s::OPTIONAL_COMPONENT, Value::OptText(default))],
        );
        assert_eq!(m.extra.description, D::Default);
    }

    /// `LivingEntity.getDimensions`: the pose's box times `minecraft:scale` —
    /// a lobby's giant fake player is clickable on its visible body, not only
    /// on the unscaled box at its feet.
    #[test]
    fn a_scaled_fake_player_gets_a_scaled_box() {
        for kind in ["minecraft:player", "minecraft:mannequin"] {
            let Some(mut m) = mob(kind) else {
                continue;
            };
            let (w, h, e) = m.dims();
            assert!(
                (w - 0.6).abs() < 1e-6 && (h - 1.8).abs() < 1e-6 && (e - 1.62).abs() < 1e-5,
                "{kind}: {w}x{h}x{e}"
            );
            m.extra.model_scale = 4.0;
            let (w, h, e) = m.dims();
            assert!(
                (w - 2.4).abs() < 1e-6 && (h - 7.2).abs() < 1e-6 && (e - 6.48).abs() < 1e-5,
                "{kind} scaled: {w}x{h}x{e}"
            );
        }
    }

    /// Vanilla 26.3 client witness (`VanillaSulfurCubeFuseOracle`, `tools/test_sulfur_cube_fuse_parity.py`).
    #[test]
    #[ignore = "run by tools/test_sulfur_cube_fuse_parity.py"]
    fn sulfur_cube_max_fuse_sets_the_client_fuse() {
        run_sulfur_cube_max_fuse_sets_the_client_fuse();
    }

    pub(in crate::game::client_side) fn run_sulfur_cube_max_fuse_sets_the_client_fuse() {
        use sunrise_sim::mobs::sulfur_cube::{client_tick, fuse, max_fuse, primed};
        let dir = std::env::var_os("SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR").unwrap_or_else(|| {
            panic!(
                "SUNRISE_SULFUR_CUBE_FUSE_WITNESS_DIR is unset; run tools/test_sulfur_cube_fuse_parity.py"
            )
        });
        let path = std::path::PathBuf::from(dir).join("sulfur_cube_fuse_client.txt");
        let witness = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| {
                panic!(
                    "failed to read witness {} (run tools/test_sulfur_cube_fuse_parity.py): {e}",
                    path.display()
                )
            })
            .replace("\r\n", "\n");
        let root =
            std::env::temp_dir().join(format!("sunrise-sulfur-receive-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("entity-types.json"),
            r#"{"minecraft:sulfur_cube":{"width":0.52,"height":0.52,"eye_height":0.325,"category":"monster","fire_immune":false,"attributes":{"minecraft:max_health":4.0}}}"#,
        )
        .unwrap();
        let attributes = &std::sync::Arc::new(
            sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": [],
                "entities": {},
            }))
            .unwrap(),
        );
        let types = sunrise_sim::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let kind = std::sync::Arc::new(types.types["minecraft:sulfur_cube"].clone());
        let mut m = None;
        let mut rows = vec![format!("# minecraft {}", sunrise_core::VANILLA_VERSION)];
        for line in witness.lines().filter(|line| !line.starts_with('#')) {
            let (op, _) = line
                .split_once(" -> ")
                .unwrap_or_else(|| panic!("bad witness row: {line}"));
            let parts: Vec<&str> = op.split_whitespace().collect();
            match parts.as_slice() {
                ["init"] => {
                    let mut mob = Mob::new(
                        1,
                        std::sync::Arc::clone(&kind),
                        sunrise_core::geometry::Vec3::ZERO,
                        0.0,
                    );
                    client_defaults(&mut mob);
                    m = Some(mob);
                }
                ["sync", id, "int", value] => {
                    let id: u8 = id.parse().unwrap();
                    let value: i32 = value.parse().unwrap();
                    apply(m.as_mut().unwrap(), &[(id, s::INT, Value::Int(value))]);
                }
                ["sync", id, "bool", value] => {
                    let id: u8 = id.parse().unwrap();
                    let value: bool = value.parse().unwrap();
                    apply(m.as_mut().unwrap(), &[(id, s::BOOLEAN, Value::Bool(value))]);
                }
                ["tick"] => {
                    client_tick(m.as_mut().unwrap());
                }
                _ => panic!("unknown witness op: {op}"),
            }
            let mob = m.as_ref().unwrap();
            rows.push(format!(
                "{op} -> {} {} {} {}",
                fuse(mob),
                max_fuse(mob),
                primed(mob),
                mob.extra.scratch[3] != 0,
            ));
        }
        let actual = rows.join("\n") + "\n";
        if let Some(path) = std::env::var_os("SUNRISE_SULFUR_CUBE_CLIENT_FUSE_OUTPUT") {
            std::fs::write(path, &actual).unwrap();
        }
        assert!(rows.len() > 1, "empty witness");
        assert_eq!(actual, witness);
    }

    /// `Pig.onSyncedDataUpdated` and `Strider.onSyncedDataUpdated`: a synced
    /// `DATA_BOOST_TIME` (index 18, after AgeableMob's two) calls
    /// `ItemBasedSteering.onSynced`. Boosting starts, `boostTime` is 0, and
    /// the total is the int (`boost()`'s minimum is 140). The next
    /// `tickBoost` then `boostFactor` is `1 + 1.15 * sin(boostTime / total * PI)`.
    #[test]
    fn a_synced_boost_starts_the_client_timer() {
        use std::{fs, sync::Arc};
        use sunrise_core::geometry::trig::Mth;
        use sunrise_sim::mobs::MobTypes;
        use sunrise_world::native_load_context::{
            DataLoadOrigin, LoadContext, LoadInputs, LoopSide,
        };

        let root = std::env::temp_dir().join(format!("sunrise-boost-sync-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("entity-types.json"),
            r#"{"minecraft:pig": {}, "minecraft:strider": {}}"#,
        )
        .unwrap();
        fs::write(
            root.join("entity-models.json"),
            r#"{"minecraft:pig#main": {}, "minecraft:strider#main": {}}"#,
        )
        .unwrap();
        fs::write(
            root.join("attributes.json"),
            format!(
                r#"{{"data_version": {}, "attributes": [], "entities": {{}}}}"#,
                sunrise_core::version::DATA_VERSION
            ),
        )
        .unwrap();
        let mut types = MobTypes::load(&root).unwrap();
        fs::remove_dir_all(root).unwrap();

        let context = LoadContext::new(
            DataLoadOrigin::ClientPreload,
            Arc::new(LoadInputs::new(Default::default(), Default::default())),
            [],
        )
        .loop_context(LoopSide::Client, []);

        // ItemBasedSteering: nextInt(841) + 140, and 1.15 in boostFactor.
        const TOTAL: i32 = 140;
        const FACTOR: f32 = 1.15;
        for (kind, steer) in [
            ("minecraft:pig", 0.225_f64),
            ("minecraft:strider", 0.55_f64),
        ] {
            let t = Arc::new(types.types.remove(kind).unwrap());
            let mut m = Mob::new(1, t, sunrise_core::geometry::Vec3::ZERO, 0.0);
            m.attrs.speed = 0.25;
            m.extra.set_saddled(true);
            m.extra.counter = 5;
            m.extra.counter2 = 3;
            // The next entry (pig variant, strider suffocating) is not the boost.
            apply(&mut m, &[(19, s::INT, Value::Int(TOTAL))]);
            assert_eq!((m.extra.counter, m.extra.counter2), (5, 3), "{kind}");
            apply(&mut m, &[(18, s::INT, Value::Int(TOTAL))]);
            assert_eq!(m.extra.counter, 0, "{kind}: onSynced sets boostTime to 0");
            assert_eq!(m.extra.counter2, TOTAL, "{kind}: the synced total");
            m.rider = Some(equine::Rider {
                steer: true,
                ..Default::default()
            });
            equine::ridden_step(&mut m, &context);
            // tickBoost post-increments, then boostFactor reads boostTime 1.
            let boost = 1.0 + FACTOR * (1.0_f32 / TOTAL as f32 * std::f32::consts::PI).msin();
            let expected = (0.25_f64 * steer) as f32 * boost;
            assert_eq!(m.extra.counter, 1, "{kind}");
            assert!(
                (m.ai.speed - expected).abs() < 1.0e-5,
                "{kind}: {} != {expected}",
                m.ai.speed
            );
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "remote_mob_data/pet_pose_meta_tests.rs"]
mod pet_pose_meta_tests;
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "remote_mob_data/witch_drink_meta_tests.rs"]
mod witch_drink_meta_tests;

use sunrise_gameplay_common::game::block_props::BlockPropsGameExt1 as _;
