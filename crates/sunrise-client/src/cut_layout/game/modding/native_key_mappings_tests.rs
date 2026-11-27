//! A mod's registered mapping read through the client host, fed by the control socket as by the
//! window: a tap is one click and one `input_key` press, each repeat one more click, a screen or
//! overlay opening drops them at once, a binding's modifier must be held, a rebinding moves the
//! mapping, a mouse press counts for a mapping on its button, and the window losing its focus
//! lets go of a held mapping.
use mod_api::prelude::ClientInputClientHost;
use mod_api::*;
use std::sync::Mutex;
use sunrise_client_gameplay::game::client_side::event_loop::EventLoopCtl;
use sunrise_gameplay_common::game::Screen;
use winit::event_loop::ControlFlow;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

struct NoLoop;
impl EventLoopCtl for NoLoop {
    fn exit(&self) {}
    fn set_control_flow(&self, _: ControlFlow) {}
}

/// The `input_key` events heard, as `key action`.
static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Registers `zoom` on Z, `boost` on Shift + Z and `grab` on the left mouse button, and hears
/// `input_key`.
struct Keys;

#[mod_api::__subscriptions]
impl Hooks for Keys {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.z"));
        r.register_key_mapping(
            KeyRegistration::new("boost", "key.keyboard.z").modifiers(ClientModifiers::new(1)),
        );
        r.register_key_mapping(KeyRegistration::new("grab", "key.mouse.left"));
    }
    fn input_key(&self, _: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        HEARD
            .lock()
            .unwrap()
            .push(format!("{} {:?}", event.key, event.action));
    }
}

fn line(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, line: &str) {
    game.replay_line(line, &NoLoop);
}
fn heard() -> Vec<String> {
    std::mem::take(&mut *HEARD.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn a_control_socket_key_counts_as_a_window_key() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_key_mappings_tests::a_control_socket_key_counts_as_a_window_key",
        "SUNRISE_MOD_KEYS_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("keys", Box::new(Keys))].into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    with_client_game(|game| {
        game.apply_settings();
        game.set_screen(Screen::Playing);
        let zoom = ClientInputClientHost::mod_key_mapping(game, "keys", "zoom").unwrap();
        let boost = ClientInputClientHost::mod_key_mapping(game, "keys", "boost").unwrap();
        let grab = ClientInputClientHost::mod_key_mapping(game, "keys", "grab").unwrap();
        heard();

        // A tap: one click, consumed once; held only while down; `input_key` hears one press
        // and one release.
        line(game, "key z tap");
        assert!(!zoom.is_down(game).unwrap());
        assert!(zoom.consume_click(game).unwrap());
        assert!(!zoom.consume_click(game).unwrap());
        assert_eq!(heard(), ["key.keyboard.z Press", "key.keyboard.z Release"]);
        // The window's tap takes the same route.
        let raw = sunrise_client::keybinds::raw_key_code(winit::keyboard::PhysicalKey::Code(
            winit::keyboard::KeyCode::KeyZ,
        ));
        let z = Some("key.keyboard.z");
        game.key_input(z, z, raw, KeyAction::Press, 0);
        assert!(zoom.is_down(game).unwrap());
        game.key_input(z, z, raw, KeyAction::Release, 0);
        assert!(!zoom.is_down(game).unwrap());
        assert!(zoom.consume_click(game).unwrap());
        assert!(!zoom.consume_click(game).unwrap());
        assert_eq!(heard(), ["key.keyboard.z Press", "key.keyboard.z Release"]);

        // Held with two repeats: three clicks.
        line(game, "key z down");
        line(game, "key z repeat");
        line(game, "key z repeat");
        assert!(zoom.is_down(game).unwrap());
        line(game, "key z up");
        assert!(!zoom.is_down(game).unwrap());
        let clicks: Vec<bool> = (0..4).map(|_| zoom.consume_click(game).unwrap()).collect();
        assert_eq!(clicks, [true, true, true, false]);
        assert_eq!(heard().len(), 4);

        // A screen opening drops the clicks at once, even closed again before anyone looks.
        line(game, "key z tap");
        game.set_screen(Screen::Inventory);
        assert!(!zoom.consume_click(game).unwrap());
        line(game, "key z tap");
        game.set_screen(Screen::Playing);
        assert!(
            !zoom.consume_click(game).unwrap(),
            "pressed behind the screen"
        );
        line(game, "key z down");
        game.set_screen(Screen::Inventory);
        game.set_screen(Screen::Playing);
        assert!(!zoom.is_down(game).unwrap());
        assert!(!zoom.consume_click(game).unwrap());
        line(game, "key z up");
        // An overlay too.
        line(game, "key z tap");
        game.open_screen_overlay("link_prompt", |game| {
            game.client.link_prompt = Some("https://example.com".into());
        });
        game.client.link_prompt = None;
        assert!(!zoom.consume_click(game).unwrap());

        // Reading is_down / consume_click while a screen is open observes live KeyMapping state
        // without suppressing Universal mappings or side-effecting release_all().
        // Opening a screen releases mappings at the transition, while a subsequent unchanged
        // build_screen/observe_screen leaves live mappings intact.
        game.set_screen(Screen::Inventory);
        game.client.mod_keys.input(
            "key.keyboard.z",
            sunrise_client::keymapping::KeyAction::Press,
            false,
            false,
            0,
        );
        game.client.mod_keys.input(
            "key.mouse.left",
            sunrise_client::keymapping::KeyAction::Press,
            true,
            false,
            0,
        );
        game.build_screen();
        game.observe_screen();
        assert!(zoom.is_down(game).unwrap());
        assert!(zoom.consume_click(game).unwrap());
        assert!(!zoom.consume_click(game).unwrap());
        assert!(zoom.is_down(game).unwrap());
        assert!(grab.consume_click(game).unwrap());
        assert!(!grab.consume_click(game).unwrap());
        game.client.mod_keys.release_all();
        game.set_screen(Screen::Playing);

        // Shift + Z counts only with shift held; Z alone counts with it too.
        line(game, "key z tap");
        assert!(!boost.consume_click(game).unwrap());
        line(game, "key shift down");
        line(game, "key z tap");
        line(game, "key shift up");
        assert!(boost.consume_click(game).unwrap());
        assert!(!boost.consume_click(game).unwrap());
        let clicks: Vec<bool> = (0..3).map(|_| zoom.consume_click(game).unwrap()).collect();
        assert_eq!(clicks, [true, true, false]);

        // Rebound in Controls to X: Z no longer counts, X does.
        game.client
            .settings
            .mods
            .set_binding("keys", "zoom", "key.keyboard.x", 0);
        game.apply_settings();
        line(game, "key z tap");
        line(game, "key x tap");
        let clicks: Vec<bool> = (0..2).map(|_| zoom.consume_click(game).unwrap()).collect();
        assert_eq!(clicks, [true, false]);

        // A mouse press counts for a mapping on its button.
        line(game, "mouse left click");
        assert!(grab.consume_click(game).unwrap());
        assert!(!grab.consume_click(game).unwrap());

        for (owner, id) in [("other", "zoom"), ("keys", "missing")] {
            assert_eq!(
                ClientInputClientHost::mod_key_mapping(game, owner, id),
                Err(ModKeyRefusal::NotRegistered)
            );
        }
    });
}

/// The window losing its focus lets go of a held mod mapping, on a key or on a mouse button, as
/// GLFW's releases let go of vanilla's (#4476); a click counted before stays to be consumed.
#[test]
#[ignore = "daily"]
fn a_window_losing_focus_lets_go_of_a_mod_mapping() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_key_mappings_tests::a_window_losing_focus_lets_go_of_a_mod_mapping",
        "SUNRISE_MOD_KEYS_FOCUS_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("keys", Box::new(Keys))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_client_game(|game| {
            game.apply_settings();
            game.set_screen(Screen::Playing);
            let zoom = ClientInputClientHost::mod_key_mapping(game, "keys", "zoom").unwrap();
            let grab = ClientInputClientHost::mod_key_mapping(game, "keys", "grab").unwrap();
            let raw = sunrise_client::keybinds::raw_key_code(winit::keyboard::PhysicalKey::Code(
                winit::keyboard::KeyCode::KeyZ,
            ));
            let z = Some("key.keyboard.z");
            game.key_input(z, z, raw, KeyAction::Press, 0);
            game.client.mod_keys.input(
                "key.mouse.left",
                sunrise_client::keymapping::KeyAction::Press,
                true,
                false,
                0,
            );
            assert!(zoom.is_down(game).unwrap());
            assert!(grab.is_down(game).unwrap());

            game.handle_window_event_inner(
                &NoLoop,
                winit::event::WindowEvent::Focused(false),
                None,
                None,
            );
            assert!(!zoom.is_down(game).unwrap());
            assert!(!grab.is_down(game).unwrap());
            assert!(zoom.consume_click(game).unwrap());
            assert!(!zoom.consume_click(game).unwrap());
            assert!(grab.consume_click(game).unwrap());
            assert!(!grab.consume_click(game).unwrap());
        });
    });
}

/// A mod rebinds its own mapping as Controls does: the new key counts from the next input, the
/// binding is saved, a reset gives the default back, and each refusal changes nothing.
#[test]
#[ignore = "daily"]
fn a_mod_rebinds_and_resets_its_own_mapping() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_key_mappings_tests::a_mod_rebinds_and_resets_its_own_mapping",
        "SUNRISE_MOD_KEYS_REBIND_CHILD",
        &["game"],
    ) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sunrise-mod-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The settings save to options.txt in the player folder and sunrise-mods.txt in the game
    // folder (the working directory): both are this child process's temporary folder.
    std::env::set_current_dir(&dir).unwrap();
    sunrise_level::logging::set_player_dir(dir.clone());
    // The saved mod list, as the next start reads it: mod `keys`'s binding of `id`.
    let saved = |id: &str| {
        sunrise_mod_package::mod_list::ModList::load(&dir).binding("keys", id, ("default", 99))
    };
    let mods: Mods = vec![ModEntry::new("keys", Box::new(Keys))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_client_game(|game| {
            game.apply_settings();
            game.set_screen(Screen::Playing);
            let zoom = ClientInputClientHost::mod_key_mapping(game, "keys", "zoom").unwrap();
            let boost = ClientInputClientHost::mod_key_mapping(game, "keys", "boost").unwrap();
            let grab = ClientInputClientHost::mod_key_mapping(game, "keys", "grab").unwrap();
            // Rebound to X with control: Z no longer counts; X counts only with control held.
            assert_eq!(
                zoom.set_binding(game, "key.keyboard.x", ClientModifiers::new(2)),
                Ok(())
            );
            assert_eq!(
                zoom.binding(game),
                Ok(("key.keyboard.x", ClientModifiers::new(2)))
            );
            assert_eq!(
                zoom.translated_key_message(game),
                Ok(Text::literal("")
                    .append(Text::translate("key.keyboard.left.control", vec![]))
                    .append(Text::literal(" + "))
                    .append(Text::literal("X")))
            );
            line(game, "key z tap");
            line(game, "key x tap");
            assert!(!zoom.consume_click(game).unwrap());
            line(game, "key control down");
            line(game, "key x tap");
            line(game, "key control up");
            assert!(zoom.consume_click(game).unwrap());
            // Saved as Controls saves it: the next start reads exactly this binding.
            assert_eq!(saved("zoom"), ("key.keyboard.x".to_owned(), 2));

            // A modifier key bound on its own is the key, not a modifier held with itself.
            assert_eq!(
                boost.set_binding(game, "key.keyboard.left.shift", ClientModifiers::new(1)),
                Ok(())
            );
            assert_eq!(
                boost.binding(game),
                Ok(("key.keyboard.left.shift", ClientModifiers::new(0)))
            );
            assert_eq!(saved("boost"), ("key.keyboard.left.shift".to_owned(), 0));

            // Reset: the declared default is back, Z counts again, and the saved list holds no
            // binding of it any more (the default answers).
            assert_eq!(zoom.reset_binding(game), Ok(()));
            assert_eq!(
                zoom.binding(game),
                Ok(("key.keyboard.z", ClientModifiers::default()))
            );
            assert_eq!(
                zoom.translated_key_message(game),
                Ok(Text::literal("").append(Text::literal("Z")))
            );
            line(game, "key z tap");
            assert!(zoom.consume_click(game).unwrap());
            assert_eq!(saved("zoom"), ("default".to_owned(), 99));

            // Refusals change nothing, in the game or in the saved list.
            let file = || std::fs::read_to_string(dir.join("sunrise-mods.txt")).unwrap();
            let file_before = file();
            assert_eq!(
                zoom.set_binding(game, "key.keyboard.nope", ClientModifiers::default()),
                Err(ModKeyRefusal::UnknownKey)
            );
            assert_eq!(
                zoom.set_binding(game, "key.keyboard.x", ClientModifiers::new(8)),
                Err(ModKeyRefusal::UnknownModifiers)
            );
            assert_eq!(
                zoom.binding(game),
                Ok(("key.keyboard.z", ClientModifiers::default()))
            );
            assert_eq!(
                boost.binding(game),
                Ok(("key.keyboard.left.shift", ClientModifiers::default()))
            );
            assert_eq!(file(), file_before);
            // Another mod can read, rebind and reset a registered mapping without an ownership gate.
            {
                let _other = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "other",
                        hook: "menu_screen_click",
                        part: None,
                    },
                );
                assert_eq!(
                    zoom.binding(game),
                    Ok(("key.keyboard.z", ClientModifiers::default()))
                );
                assert_eq!(
                    zoom.set_binding(game, "key.keyboard.x", ClientModifiers::default()),
                    Ok(())
                );
                assert_eq!(
                    zoom.binding(game),
                    Ok(("key.keyboard.x", ClientModifiers::default()))
                );
                game.client.mod_keys.input(
                    "key.keyboard.x",
                    sunrise_client::keymapping::KeyAction::Press,
                    false,
                    false,
                    0,
                );
                assert_eq!(zoom.is_down(game), Ok(true));
                game.client.mod_keys.input(
                    "key.keyboard.x",
                    sunrise_client::keymapping::KeyAction::Release,
                    false,
                    false,
                    0,
                );
                assert_eq!(zoom.is_down(game), Ok(false));
                assert_eq!(zoom.consume_click(game), Ok(true));
                assert_eq!(zoom.consume_click(game), Ok(false));
                assert_eq!(boost.reset_binding(game), Ok(()));
                assert_eq!(
                    boost.binding(game),
                    Ok(("key.keyboard.z", ClientModifiers::new(1)))
                );
            }
            assert_eq!(saved("zoom"), ("key.keyboard.x".to_owned(), 0));
            assert_eq!(saved("boost"), ("default".to_owned(), 99));
            // Inside `keys`'s own hook, the same call goes through.
            {
                let _own = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "keys",
                        hook: "menu_screen_click",
                        part: None,
                    },
                );
                assert_eq!(
                    zoom.set_binding(game, "key.keyboard.y", ClientModifiers::new(0)),
                    Ok(())
                );
            }
            assert_eq!(saved("zoom"), ("key.keyboard.y".to_owned(), 0));

            // A mouse button and "no key" are bindings too.
            assert_eq!(
                grab.set_binding(game, "key.mouse.right", ClientModifiers::new(0)),
                Ok(())
            );
            assert_eq!(saved("grab"), ("key.mouse.right".to_owned(), 0));
            assert_eq!(
                grab.set_binding(game, "key.keyboard.unknown", ClientModifiers::new(0)),
                Ok(())
            );
            assert_eq!(saved("grab"), ("key.keyboard.unknown".to_owned(), 0));

            // A registry replacement can reuse an index, but not the authority of its old handle.
            game.client
                .mod_keys
                .bind([sunrise_client::keymapping::ModBinding {
                    owner: "keys",
                    id: "replacement",
                    input: "key.keyboard.n".into(),
                    modifiers: 0,
                    context: sunrise_client::keymapping::MappingContext::InGame,
                }]);
            let replacement =
                ClientInputClientHost::mod_key_mapping(game, "keys", "replacement").unwrap();
            line(game, "key n tap");
            let file_before = file();
            assert_eq!(zoom.binding(game), Err(ModKeyRefusal::Stale));
            assert_eq!(zoom.is_down(game), Err(ModKeyRefusal::Stale));
            assert_eq!(zoom.translated_key_message(game), Err(MappingError::Stale));
            assert_eq!(zoom.consume_click(game), Err(ModKeyRefusal::Stale));
            assert_eq!(
                zoom.set_binding(game, "key.keyboard.x", ClientModifiers::default()),
                Err(ModKeyRefusal::Stale)
            );
            assert_eq!(zoom.reset_binding(game), Err(ModKeyRefusal::Stale));
            assert_eq!(
                replacement.binding(game),
                Ok(("key.keyboard.n", ClientModifiers::default()))
            );
            assert_eq!(replacement.consume_click(game), Ok(true));
            assert_eq!(replacement.consume_click(game), Ok(false));
            assert_eq!(file(), file_before);
        });
    });
    let _ = std::fs::remove_dir_all(&dir);
}
