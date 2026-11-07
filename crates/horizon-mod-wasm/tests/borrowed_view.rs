//! A real guest updates a borrowed block-entity view through the generated codec.
use horizon_mod_wasm::{RuntimeConfig, WasmPackage};

struct Server;
impl mod_api::Host for Server {}
mod_api::impl_host_areas!(Server);
impl mod_api::ServerHost for Server {}
mod_api::impl_server_host_areas!(Server);

#[test]
#[ignore = "needs the benchmark guest built to mod.wasm (wasm-bench job)"]
fn guest_block_entity_tick_updates_host_fields_without_replacing_identity() {
    let path = std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_BENCH").unwrap());
    let loaded = horizon_mod_wasm::load(
        &RuntimeConfig::default(),
        &std::fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
        vec![WasmPackage {
            manifest: horizon_mod_package::Package::read(&path).unwrap().manifest,
            module: horizon_mod_package::Package::read_wasm(&path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    mod_api::__private::install_parts(&loaded.mods).unwrap();
    mod_api::__private::install(loaded.mods.into());
    mod_api::__private::mark_game_thread();
    let mut entity = mod_api::ModBlockEntity::new("bench_mod:counter".into(), Default::default());
    entity.items.push(None);
    let revision = entity.revision();
    mod_api::dispatch_block_entity_tick(&mut Server, [1, 65, 2], &mut entity);
    assert_eq!(
        entity.nbt.get(&"counter".into()),
        Some(&mod_api::Tag::Int(17))
    );
    assert_eq!(entity.data, [23]);
    assert!(entity.items.is_empty());
    assert_eq!(entity.id(), "bench_mod:counter");
    assert_ne!(entity.revision(), revision);
    let revision = entity.revision();
    // The same returned fields are a no-op, not a new host identity or revision.
    mod_api::dispatch_block_entity_tick(&mut Server, [1, 65, 2], &mut entity);
    assert_eq!(entity.revision(), revision);
}
