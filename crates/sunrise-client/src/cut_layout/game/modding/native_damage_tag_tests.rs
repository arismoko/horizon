//! A hit listener reads its source's damage type tags through the hit's host
//! (`DamageSource.is(TagKey)`), against the vanilla damage registry the world bound.
use mod_api::{Flow, HitHost, Hooks, Identifier, LivingIncomingDamageEvent};
use std::sync::{Arc, Mutex};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

/// The damage type tags the listener reads, and what it read for each hit.
#[derive(Default)]
struct Reads {
    pub tags: Vec<String>,
    hits: Vec<Hit>,
}

/// One hit's damage type, its answer for each of [`Reads::tags`] in order, and its answers for
/// a tag no data defines and for a damage type the registry lacks.
struct Hit {
    pub damage_type: String,
    members: Vec<Option<bool>>,
    absent_tag: Option<bool>,
    unknown_type: Option<bool>,
}

struct Probe(Arc<Mutex<Reads>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        level: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        let tags = level.tags();
        // Keys first made here, after the world bound its tags.
        let key = |name: &str| tags.key(&Identifier::parse(name).unwrap()).unwrap().typed();
        let mut reads = self.0.lock().unwrap();
        let damage_type = event.source.damage_type;
        let members = reads
            .tags
            .iter()
            .map(|tag| tags.damage_type_in_tag(damage_type, key(tag)))
            .collect();
        let fire = key("minecraft:is_fire");
        reads.hits.push(Hit {
            damage_type: damage_type.to_owned(),
            members,
            absent_tag: tags.damage_type_in_tag(damage_type, key("test:no_such_damage_tag")),
            unknown_type: tags.damage_type_in_tag("test:no_such_damage_type", fire),
        });
        // Cancel, so no hurt cooldown keeps the next hit from the hook.
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn a_hit_listener_reads_its_damage_type_tags_as_bound() {
    const CASE: &str = "SUNRISE_TEST_DAMAGE_TYPE_TAGS";
    if run_fresh_cases(
        "cut_layout::game::native_tags::native_damage_tag_tests::a_hit_listener_reads_its_damage_type_tags_as_bound",
        CASE,
        &["hits"],
    ) {
        return;
    }
    const TYPES: [&str; 3] = [
        "minecraft:arrow",
        "minecraft:mob_attack",
        "minecraft:in_fire",
    ];
    let reads = Arc::new(Mutex::new(Reads::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(reads.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let catalog = game.health_resources.damage.clone();
            let mut tags: Vec<String> = catalog.tags().map(str::to_owned).collect();
            tags.sort();
            reads.lock().unwrap().tags = tags.clone();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            bind_local_player(game, 7, 7);
            game.set_game_mode(GameMode::Survival);
            for damage_type in TYPES {
                game.hurt_player_from(
                    2.0,
                    None,
                    damage_type,
                    sunrise_gameplay_common::game::creatures::KnownCause::None,
                    false,
                );
            }
            let reads = reads.lock().unwrap();
            let seen: Vec<_> = reads
                .hits
                .iter()
                .map(|hit| hit.damage_type.as_str())
                .collect();
            assert_eq!(seen, TYPES);
            for hit in &reads.hits {
                let id = catalog.id(&hit.damage_type).unwrap();
                let expected: Vec<_> = tags
                    .iter()
                    .map(|tag| Some(catalog.in_tag(id, tag)))
                    .collect();
                assert_eq!(hit.members, expected, "{}", hit.damage_type);
                assert_eq!(hit.absent_tag, Some(false), "{}", hit.damage_type);
                assert_eq!(hit.unknown_type, None, "{}", hit.damage_type);
            }
            let member = |damage_type: &str, tag: &str| {
                let hit = reads
                    .hits
                    .iter()
                    .find(|hit| hit.damage_type == damage_type)
                    .unwrap();
                hit.members[tags.iter().position(|name| name == tag).unwrap()]
            };
            assert_eq!(
                member("minecraft:arrow", "minecraft:is_projectile"),
                Some(true)
            );
            assert_eq!(
                member("minecraft:mob_attack", "minecraft:is_projectile"),
                Some(false)
            );
            assert_eq!(member("minecraft:in_fire", "minecraft:is_fire"), Some(true));
            assert_eq!(member("minecraft:arrow", "minecraft:is_fire"), Some(false));
        });
    });
}
