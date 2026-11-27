use mod_api::ServerHost;
use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_sim::mobs::Damage;

/// What the mod read through its handles, of one question.
#[derive(Debug, PartialEq)]
pub(crate) struct Heard {
    pub entity: Option<String>,
    pub attacker: Option<String>,
    pub(in crate::game) damage_type: String,
    pub(in crate::game) enchantment: String,
    pub(in crate::game) level: u32,
}
/// Answers every question with level `self.1`.
pub(crate) struct Answer(pub(crate) Arc<Mutex<Vec<Heard>>>, pub(crate) u32);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Answer {
    fn enchanted_entity_loot(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::EnchantedEntityLootEvent<'_>,
    ) {
        self.0.lock().unwrap().push(Heard {
            entity: event
                .entity
                .entity()
                .kind(host)
                .and_then(|kind| kind.id(host))
                .map(|name| name.as_str().to_owned()),
            attacker: event
                .attacker
                .and_then(|a| a.kind(host)?.id(host))
                .map(|name| name.as_str().to_owned()),
            damage_type: event.source.damage_type.to_owned(),
            enchantment: event.enchantment.to_string(),
            level: event.level,
        });
        event.level = self.1;
    }
}

pub(crate) fn listening(seen: &Arc<Mutex<Vec<Heard>>>, level: u32) -> mod_api::Mods {
    let answer = Box::new(Answer(Arc::clone(seen), level));
    vec![mod_api::ModEntry::new("enchanted-loot", answer)].into()
}
#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::enchanted_entity_loot_tests::zombie;
pub(crate) fn dropped(game: &sunrise_gameplay::game::ServerGame<'_>, item: &str) -> u32 {
    let drops = game.dropped.iter().filter(|d| &*d.stack.item == item);
    drops.map(|d| d.stack.count).sum()
}
pub(crate) fn heard(entity: &str, attacker: &str, damage_type: &str, level: u32) -> Heard {
    Heard {
        entity: Some(entity.into()),
        attacker: Some(attacker.into()),
        damage_type: damage_type.into(),
        enchantment: "minecraft:looting".into(),
        level,
    }
}

/// No attacker at all: `getAttacker` is `@Nullable`, and the read is
/// asked from a level of 0.
#[test]
#[ignore = "daily"]
fn a_death_with_no_attacker_is_asked_at_level_0() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = listening(&seen, 1000);
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.load_server_loot();
            let victim = zombie(game, Damage::fall());
            game.mob_die(&victim);
            assert_eq!(
                *seen.lock().unwrap(),
                [Heard {
                    attacker: None,
                    ..heard("minecraft:zombie", "", "minecraft:fall", 0)
                }]
            );
            assert!(dropped(game, "minecraft:rotten_flesh") > 2);
        });
    });
}

/// A listener that leaves the level as it came changes nothing: the same
/// deaths drop the same as with no mod, one question each.
#[test]
#[ignore = "daily"]
fn an_answer_of_the_same_level_leaves_the_drops_alone() {
    let deaths = |mods: mod_api::Mods| {
        let mut counts = Vec::new();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                for _ in 0..8 {
                    let victim = zombie(game, Damage::mob(0, [0.0; 3]));
                    game.mob_die(&victim);
                    counts.push(dropped(game, "minecraft:rotten_flesh"));
                }
            });
        });
        counts
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listening = vec![mod_api::ModEntry::new(
        "enchanted-loot",
        Box::new(Answer(Arc::clone(&seen), 0)),
    )]
    .into();
    assert_eq!(deaths(listening), deaths(Vec::new().into()));
    assert_eq!(seen.lock().unwrap().len(), 8);
}
