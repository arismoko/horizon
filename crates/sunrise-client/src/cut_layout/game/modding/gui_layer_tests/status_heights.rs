//! A layer reads the status columns' heights as they stand when it starts (NeoForge's
//! `Hud.leftHeight`, `rightHeight`), and one that sets them moves the rows drawn after it.

use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{
    ClientHost, GuiLayerPlacement, Hooks, ModEntry, RegisterGuiLayersEvent, RegistrationContext,
};
use std::sync::Mutex;
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
use sunrise_gameplay_common::game::{Screen, execution_role::ExecutionRole};
use sunrise_sim::motion::PlayerMotion;

static SEEN: Mutex<Vec<(&str, Option<[i32; 2]>)>> = Mutex::new(Vec::new());

struct Probe {
    replace_armor: bool,
    /// The layer after armour raises the right column to 60, as a mod that draws a row there.
    set_right: bool,
    /// A wrapper on the food row sets both columns (left 70, right 50) before `draw_previous`,
    /// so the food row it wraps draws from 50.
    wrap_food: bool,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
        use GuiLayerPlacement::*;
        let set_right = self.set_right;
        for (id, placement) in [
            (
                "heights:before_health",
                Below("minecraft:player_health".into()),
            ),
            (
                "heights:after_health",
                Above("minecraft:player_health".into()),
            ),
            ("heights:after_armor", Above("minecraft:armor_level".into())),
            ("heights:after_food", Above("minecraft:food_level".into())),
            ("heights:after_air", Above("minecraft:air_level".into())),
        ] {
            assert!(event.layers.register(
                id.into(),
                placement,
                Box::new(move |host: &dyn ClientHost, _| {
                    SEEN.lock().unwrap().push((id, host.gui_status_heights()));
                    if set_right && id == "heights:after_armor" {
                        let [left, _] = host.gui_status_heights().unwrap();
                        assert!(host.set_gui_status_heights([left, 60]));
                        assert_eq!(host.gui_status_heights(), Some([left, 60]));
                    }
                }),
            ));
        }
        if self.wrap_food {
            assert!(event.layers.wrap_layer(
                "minecraft:food_level",
                Box::new(|ctx| {
                    let host = ctx.host();
                    SEEN.lock()
                        .unwrap()
                        .push(("heights:wrap_food", host.gui_status_heights()));
                    assert!(host.set_gui_status_heights([70, 50]));
                    ctx.draw_previous();
                }),
            ));
        }
        if self.replace_armor {
            assert!(
                event
                    .layers
                    .replace_layer("minecraft:armor_level", Box::new(|_, _| {}))
            );
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_layer_reads_the_status_heights_the_vanilla_rows_before_it_left() {
    const CASE: &str = "SUNRISE_GUI_STATUS_HEIGHTS_FIXTURE";
    if run_fresh_cases(
        "cut_layout::game::client_side::gui_layer_tests::status_heights::a_layer_reads_the_status_heights_the_vanilla_rows_before_it_left",
        CASE,
        &["vanilla", "replace_armor", "set_right", "wrap_food"],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let replace_armor = case == "replace_armor";
    let set_right = case == "set_right";
    let wrap_food = case == "wrap_food";
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![ModEntry::new(
                "heights",
                Box::new(Probe {
                    replace_armor,
                    set_right,
                    wrap_food,
                }),
            )]
            .into(),
        );
        sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
            ExecutionRole::Client,
        );
        game.client.title.0 = None;
        game.client.screen = Screen::Playing;
        game.client.chat_ui.hide_gui = false;
        super::set_fixture_mode(game, GameMode::Survival);
        // One row of hearts, one point of armour, full food and depleted air above water: the
        // health, armour, food and air rows each take 10 pixels.
        let mut player = PlayerMotion::new(game.motion, Vec3::new(2.0, 64.0, 2.0)).unwrap();
        player.set_health(12.0, false).unwrap();
        player
            .set_modifier("minecraft:armor", "heights:armor", 2.0, "add_value", true)
            .unwrap();
        player.air = 181;
        game.player = Some(player);
        assert_eq!(game.gui_status_heights(), None);
        assert!(!game.set_gui_status_heights([1, 1]));
        game.hud_layers([640, 480]);
        let after_armor = if replace_armor { [49, 39] } else { [59, 39] };
        let mut expected = vec![
            ("heights:before_health", Some([39, 39])),
            ("heights:after_health", Some([49, 39])),
            ("heights:after_armor", Some(after_armor)),
        ];
        if wrap_food {
            // The wrapper reads the heights the food row would start from, then sets both
            // columns: the food row it wraps draws from 50, and the left column stays at 70.
            expected.push(("heights:wrap_food", Some([59, 39])));
            expected.push(("heights:after_food", Some([70, 60])));
            expected.push(("heights:after_air", Some([70, 70])));
        } else {
            let [left, _] = after_armor;
            // Food and air push 10 each from wherever the right column stands.
            let right = if set_right { 60 } else { 39 };
            expected.push(("heights:after_food", Some([left, right + 10])));
            expected.push(("heights:after_air", Some([left, right + 20])));
        }
        assert_eq!(*SEEN.lock().unwrap(), expected);
        assert_eq!(game.gui_status_heights(), None);
    });
}
