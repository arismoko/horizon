use std::sync::{Arc, Mutex};

// Native mod registries are process-global. Each parallel test owns a fresh process
// and temporary working directory, while retaining the normal test runner parallelism.
pub(crate) fn isolated() -> bool {
    const FLAG: &str = "SUNRISE_DEATH_WEAPON_CHILD";
    if std::env::var_os(FLAG).is_some() {
        return false;
    }
    let scratch = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap())
        .canonicalize()
        .unwrap();
    let thread = std::thread::current();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            thread.name().unwrap(),
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(FLAG, "1")
        .env("SUNRISE_TEST_RESOURCES", resources)
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1");
    for key in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        command.env(key, scratch.path().join(key));
    }
    assert!(command.status().unwrap().success());
    true
}

pub(crate) struct WeaponProbe {
    pub(crate) expected: Option<mod_api::ItemStack>,
    pub(crate) expected_source: Option<(&'static str, i32)>,
    pub(crate) swap: Option<mod_api::ItemStack>,
    pub(crate) seen: Arc<Mutex<usize>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for WeaponProbe {
    fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_, '_>,
    ) {
        assert!(event.context.tool.is_none());
        if let Some(expected) = self.expected_source {
            let source = event.context.damage_source.unwrap();
            assert_eq!((source.damage_type, source.causing), expected);
        }
        assert_eq!(event.context.weapon_item(host), self.expected);
        if let Some(axe) = &self.swap {
            let source = event.context.damage_source.unwrap();
            let living = host
                .living(host.entity(source.causing as u32).unwrap())
                .unwrap();
            assert!(
                living
                    .set_hand_item(host, mod_api::Hand::Main, Some(axe.clone()))
                    .unwrap()
            );
            assert_eq!(event.context.weapon_item(host), Some(axe.clone()));
        }
        *self.seen.lock().unwrap() += 1;
    }
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::death_weapon::tests::weapon_case;

#[test]
#[ignore = "daily"]
fn death_weapon_a_longbow_power_ii_survives_hand_change() {
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case("minecraft:bow", "minecraft:power", 2, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_b_crossbow_piercing_i() {
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case("minecraft:crossbow", "minecraft:piercing", 1, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_c_trident_loyalty_ii() {
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case("minecraft:trident", "minecraft:loyalty", 2, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_d_sharpness_v_sword_reads_live_axe() {
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case(
            "minecraft:diamond_sword",
            "minecraft:sharpness",
            5,
            false,
            true,
        );
    });
}
