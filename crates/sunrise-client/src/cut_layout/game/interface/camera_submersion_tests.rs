//! The mods replace the fluid the camera is in (`camera_submersion`), each listener seeing the
//! answer of the one before, and the camera's fluid readers follow the last answer.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{CameraSubmersionEvent, ClientHost, Hooks, ModEntry};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};
use sunrise_client::{camera::CameraFluid, fog::FogType};
use sunrise_level::chunk::Chunk;

type Heard = Arc<Mutex<Vec<([f64; 3], FogType)>>>;

/// A detached camera's mod: no water fog in water, and lava forced from y 70 up.
struct Dry(Heard);
#[mod_api::__subscriptions]
impl Hooks for Dry {
    fn camera_submersion(&self, _: &dyn ClientHost, event: &mut CameraSubmersionEvent<'_>) {
        self.0.lock().unwrap().push((event.camera, *event.fog_type));
        match *event.fog_type {
            FogType::Water => *event.fog_type = FogType::Atmospheric,
            FogType::Atmospheric if event.camera[1] >= 70.0 => *event.fog_type = FogType::Lava,
            _ => {}
        }
    }
}

/// Hears what the listener before it left, and keeps it.
struct After(Heard);
#[mod_api::__subscriptions]
impl Hooks for After {
    fn camera_submersion(&self, _: &dyn ClientHost, event: &mut CameraSubmersionEvent<'_>) {
        self.0.lock().unwrap().push((event.camera, *event.fog_type));
    }
}

#[test]
fn a_panicking_submersion_listener_preserves_the_incoming_fog() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Panics(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Panics {
        #[listener(priority = 10)]
        fn camera_submersion(&self, _: &dyn ClientHost, event: &mut CameraSubmersionEvent<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
            *event.fog_type = FogType::Lava;
            panic!("fog listener failed after changing the value");
        }
    }

    let heard = Heard::default();
    let attempts = Arc::new(AtomicUsize::new(0));
    let mods = vec![
        ModEntry::new("broken-fog", Box::new(Panics(attempts.clone()))),
        ModEntry::new("observer", Box::new(After(heard.clone()))),
    ];
    with_mods(mods.into(), || {
        let host = mod_api::testing::FakeHost::new();
        let position = [4.5, 64.5, 4.5];
        for incoming in [FogType::Water, FogType::Atmospheric] {
            let mut fog = incoming;
            mod_api::dispatch_camera_submersion(&host, position, &mut fog);
            assert_eq!(fog, incoming);
        }
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert_eq!(
            *heard.lock().unwrap(),
            [(position, FogType::Water), (position, FogType::Atmospheric)]
        );
        assert_eq!(
            take_hook_panics(),
            [HookPanicReport {
                mod_id: "broken-fog",
                hook: "camera_submersion",
            }]
        );
    });
}

fn water_at_4_64_4(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let water = game
        .pack
        .assets
        .registry
        .default_state("minecraft:water")
        .unwrap();
    game.world.set(4, 64, 4, water).unwrap();
}

#[test]
#[ignore = "daily"]
fn listeners_replace_the_fluid_the_camera_is_in() {
    use FogType::*;
    let (wet, dry, high) = ([4.5, 64.5, 4.5], [4.5, 66.5, 4.5], [4.5, 70.5, 4.5]);
    with_client_game(|game| {
        water_at_4_64_4(game);
        assert_eq!(
            game.camera_fog_type(wet),
            Water,
            "no listener: vanilla's answer"
        );
        assert_eq!(game.camera_fog_type(high), Atmospheric);
    });
    let (first, second) = (Heard::default(), Heard::default());
    let mods: mod_api::Mods = vec![
        ModEntry::new("dry", Box::new(Dry(Arc::clone(&first)))),
        ModEntry::new("after", Box::new(After(Arc::clone(&second)))),
    ]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            water_at_4_64_4(game);
            assert_eq!(game.camera_fog_type(wet), Atmospheric, "water cleared");
            assert_eq!(
                game.camera_fluid(wet),
                CameraFluid::None,
                "the FOV reads it too"
            );
            assert_eq!(game.camera_fog_type(dry), Atmospheric);
            assert_eq!(game.camera_fog_type(high), Lava, "lava forced");
        });
    });
    assert_eq!(
        *first.lock().unwrap(),
        [
            (wet, Water),
            (wet, Water),
            (dry, Atmospheric),
            (high, Atmospheric)
        ],
        "once per read, with the game's answer and the position asked"
    );
    assert_eq!(
        *second.lock().unwrap(),
        [
            (wet, Atmospheric),
            (wet, Atmospheric),
            (dry, Atmospheric),
            (high, Lava)
        ],
        "the next listener sees the earlier one's value"
    );
}
