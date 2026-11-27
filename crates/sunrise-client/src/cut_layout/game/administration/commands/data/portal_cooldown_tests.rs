use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::server_side::commands::data::DataRef,
    sunrise_gameplay_common::game::commands_common::Target,
};

#[derive(serde::Deserialize)]
struct Witness {
    minecraft: String,
    inputs: Inputs,
    expected: Expected,
}

#[derive(serde::Deserialize)]
struct Inputs {
    initial_cooldown: i32,
    merged_cooldown: i32,
}

#[derive(serde::Deserialize)]
struct Expected {
    after_summon: i32,
    after_first_tick: i32,
    after_chunk_reload: i32,
    after_reload_and_tick: i32,
    after_data_merge: i32,
    after_merge_and_tick: i32,
    after_data_remove: i32,
    plain_after_summon: i32,
    plain_after_first_tick: i32,
}

const WITNESS_ENV: &str = "SUNRISE_MOB_PORTAL_COOLDOWN_WITNESS";

fn load_witness() -> Witness {
    let raw_path = std::env::var_os(WITNESS_ENV).unwrap_or_else(|| {
        panic!(
            "{WITNESS_ENV} is unset; run tools/test_mob_portal_cooldown_parity.py to record the witness"
        )
    });
    let mut path = std::path::PathBuf::from(raw_path);
    if path.is_dir() {
        path.push("mob_portal_cooldown.json");
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (record via tools/test_mob_portal_cooldown_parity.py): {e}",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("valid 26.3 mob portal cooldown witness JSON")
}

fn cooldown(game: &mut sunrise_gameplay::game::ServerGame<'_>, target: Target) -> Option<Tag> {
    game.data_of(&DataRef::Entity(target))
        .unwrap()
        .get("PortalCooldown")
        .cloned()
}

/// #2387 (vanilla 26.3): a mob's live PortalCooldown across tick, save/load, and /data edits matches the vanilla 26.3 witness `tools/test_mob_portal_cooldown_parity.py` records.
#[test]
#[ignore = "run by tools/test_mob_portal_cooldown_parity.py"]
fn a_mob_saves_its_live_portal_cooldown() {
    run_a_mob_saves_its_live_portal_cooldown();
}

pub(crate) fn run_a_mob_saves_its_live_portal_cooldown() {
    let witness = load_witness();
    assert_eq!(witness.minecraft, "26.3");
    let expected = &witness.expected;
    with_game(|game| {
        // A player's tickets make the chunks around the origin entity-ticking.
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        let initial = witness.inputs.initial_cooldown;
        let pig = game
            .summon_with_nbt(
                "minecraft:pig",
                [0.5, 64.0, 0.5],
                &format!("{{NoAI:1b,NoGravity:1b,PortalCooldown:{initial}}}"),
            )
            .unwrap()
            .unwrap();
        assert_eq!(cooldown(game, pig), Some(Tag::Int(expected.after_summon)));
        game.handle_entity_portals();
        assert_eq!(
            cooldown(game, pig),
            Some(Tag::Int(expected.after_first_tick))
        );

        // The chunk save writes the live cooldown, and the reloaded pig counts on from there.
        let Target::Mob(id) = pig else {
            panic!("{pig:?}")
        };
        let i = game.mobs.iter().position(|m| m.id == id).unwrap();
        let saved = game.mob_tree_nbt(i);
        assert_eq!(
            saved.get("PortalCooldown"),
            Some(&Tag::Int(expected.after_chunk_reload))
        );
        game.mobs.retain(|m| m.id != id);
        game.forget_entity(id);
        let reloaded = Target::Mob(game.load_mob(saved, None).unwrap());
        assert_eq!(
            cooldown(game, reloaded),
            Some(Tag::Int(expected.after_chunk_reload))
        );
        game.handle_entity_portals();
        assert_eq!(
            cooldown(game, reloaded),
            Some(Tag::Int(expected.after_reload_and_tick))
        );

        // `/data merge` is `Entity.load` on the live mob.
        let mut merged = game.data_of(&DataRef::Entity(reloaded)).unwrap();
        if let Tag::Compound(fields) = &mut merged {
            fields.insert(
                "PortalCooldown".into(),
                Tag::Int(witness.inputs.merged_cooldown),
            );
        }
        game.apply_entity_nbt(reloaded, merged).unwrap();
        assert_eq!(
            cooldown(game, reloaded),
            Some(Tag::Int(expected.after_data_merge))
        );
        game.handle_entity_portals();
        assert_eq!(
            cooldown(game, reloaded),
            Some(Tag::Int(expected.after_merge_and_tick))
        );

        // `/data remove` clears `PortalCooldown` before `Entity.load` runs on the live mob.
        let mut removed = game.data_of(&DataRef::Entity(reloaded)).unwrap();
        if let Tag::Compound(fields) = &mut removed {
            fields.remove(&"PortalCooldown".into());
        }
        game.apply_entity_nbt(reloaded, removed).unwrap();
        assert_eq!(
            cooldown(game, reloaded),
            Some(Tag::Int(expected.after_data_remove))
        );

        let plain = game
            .summon_with_nbt("minecraft:pig", [4.5, 64.0, 0.5], "{NoAI:1b,NoGravity:1b}")
            .unwrap()
            .unwrap();
        assert_eq!(
            cooldown(game, plain),
            Some(Tag::Int(expected.plain_after_summon))
        );
        game.handle_entity_portals();
        assert_eq!(
            cooldown(game, plain),
            Some(Tag::Int(expected.plain_after_first_tick))
        );
    });
}
