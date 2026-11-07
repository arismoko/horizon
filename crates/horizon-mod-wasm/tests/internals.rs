//! A real compiled guest view, loaded through the same package and erased raw route as the game.
#[path = "internals/host.rs"]
mod host;
use host::{Game, Record};

#[test]
#[ignore = "needs tests/internals/counter built to mod.wasm (CI's wasm-mods job)"]
fn a_generated_guest_view_reads_writes_calls_and_commits_a_raw_listener() {
    let point = mod_points::PointId::HandleChatInput;
    let place = point as usize;
    let table = host::table();
    horizon_mod_wasm::set_internals(table);
    let path = |var: &str| std::path::PathBuf::from(std::env::var(var).expect(var));
    let package =
        path("HORIZON_WASM_INTERNALS").join("counter/target/horizon-mod/internals_counter.hmod");
    let loaded = horizon_mod_wasm::load(
        &horizon_mod_wasm::RuntimeConfig::default(),
        &std::fs::read(path("HORIZON_WASM_STD")).unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest: horizon_mod_package::Package::read(&package)
                .unwrap()
                .manifest,
            module: horizon_mod_package::Package::read_wasm(&package).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    for listening in loaded.listening {
        listening.register().unwrap();
    }
    mod_points::__private::seal();
    let mut game = Game {
        left: 1,
        right: 10,
        hidden: vec![99],
        refused_sets: 0,
        reads: 0,
    };
    let mut raw = Record {
        message: "before".into(),
        cancel: false,
    };
    mod_points::__private::fire_raw(point, "handle_chat_input", &mut |kind| {
        let mod_points::__private::RawKind::Indexed(callback, index) = kind else {
            panic!("a wasm raw listener")
        };
        // SAFETY: the descriptor above describes exactly these two live fixture roots.
        unsafe {
            callback(place, index, (&raw mut game).cast(), (&raw mut raw).cast());
        }
    });
    assert_eq!(
        game,
        Game {
            left: 8,
            right: 13,
            hidden: vec![99],
            refused_sets: 0,
            reads: 1,
        }
    );
    assert_eq!(raw.message, "proof21");
    assert!(raw.cancel);
}
