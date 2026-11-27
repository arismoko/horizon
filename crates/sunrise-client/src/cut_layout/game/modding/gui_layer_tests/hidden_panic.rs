//! A status replacement must use the shared retained HUD callback boundary.

use super::{AIR_LEVEL, log, take};
use mod_api::{Hooks, ModEntry, RegisterGuiLayersEvent, RegistrationContext};
use std::sync::Mutex;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

#[test]
#[ignore = "daily"]
fn a_panicking_hidden_status_replacement_leaves_other_rows() {
    if run_fresh_cases(
        "cut_layout::game::client_side::gui_layer_tests::hidden_panic::a_panicking_hidden_status_replacement_leaves_other_rows",
        "SUNRISE_HIDDEN_STATUS_PANIC_FIXTURE",
        &["run"],
    ) {
        return;
    }

    const FAILED: u32 = 0xFF31_3131;
    const SIBLING: u32 = 0xFF32_3232;
    const PANIC: &str = "status paint fixture panic";
    struct VehicleReplacement;
    struct AirReplacement;

    #[mod_api::__subscriptions]
    impl Hooks for VehicleReplacement {
        fn register_gui_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterGuiLayersEvent<'_>,
        ) {
            assert!(event.layers.replace_layer(
                "minecraft:vehicle_health",
                Box::new(|_, draw| {
                    log("mount replacement");
                    draw.fill([80.0, 100.0, 10.0, 4.0], FAILED);
                    panic!("{PANIC}");
                }),
            ));
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for AirReplacement {
        fn register_gui_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterGuiLayersEvent<'_>,
        ) {
            assert!(event.layers.replace_layer(
                AIR_LEVEL,
                Box::new(|_, draw| {
                    log("air replacement");
                    draw.fill([80.0, 125.0, 10.0, 4.0], SIBLING);
                }),
            ));
        }
    }

    static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    struct Diagnostics;
    impl log::Log for Diagnostics {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() == log::Level::Error
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                ERRORS.lock().unwrap().push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }
    log::set_logger(&Diagnostics).unwrap();
    log::set_max_level(log::LevelFilter::Error);
    with_client_game(|game| {
        mod_platform::__private::install(
            vec![
                ModEntry::new("bad-vehicle-painter", Box::new(VehicleReplacement)),
                ModEntry::new("air-painter", Box::new(AirReplacement)),
            ]
            .into(),
        );
        sunrise_client_gameplay::game::client_side::mods_client::register_gui_layers(
            ExecutionRole::Client,
        );
        game.client.title.0 = None;
        game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
        game.client.chat_ui.hide_gui = true;
        for (frame, expected) in [
            &["mount replacement", "air replacement"][..],
            &["air replacement"],
        ]
        .into_iter()
        .enumerate()
        {
            let batch = game.hud([640, 480]);
            assert_eq!(take().as_slice(), expected);
            assert_eq!(batch.vertices.iter().any(|v| v.color == FAILED), frame == 0);
            assert!(batch.vertices.iter().any(|v| v.color == SIBLING));
            assert_eq!(
                game.client.chat_ui.room.bottom(),
                if frame == 0 { 100.0 } else { 125.0 }
            );
            if frame == 0 {
                // The shared queue retains the failure under F1.
                let [
                    sunrise_client_gameplay::game::toast_manager::VisibleToast {
                        toast:
                            sunrise_client_gameplay::game::toast_manager::Toast::Diagnostic {
                                toast,
                                ..
                            },
                        ..
                    },
                ] = game.client.toast_manager.visible.as_slice()
                else {
                    panic!("expected one mandatory mod error toast");
                };
                assert_eq!(
                    toast.message.as_ref().unwrap().component(),
                    serde_json::json!({
                        "translate": "sunrise.mods.hook_panic.description",
                        "with": [{"text": "bad-vehicle-painter"}, {"text": "gui_layer_draw"}],
                    })
                );
            }
        }
    });
    let errors = ERRORS.lock().unwrap();
    let reports: Vec<_> = errors.iter().filter(|msg| msg.contains(PANIC)).collect();
    assert_eq!(reports.len(), 1, "{errors:?}");
    assert!(reports[0].contains("bad-vehicle-painter"));
}
