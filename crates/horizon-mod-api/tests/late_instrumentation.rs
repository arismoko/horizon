//! The engine can dispatch without the platform ever loading mods.
#[test]
fn dispatch_before_initialization_is_inert_and_seals_the_socket() {
    let guard = mod_vocab::__private::enter("unloaded", "client_tick_pre");
    assert_eq!(mod_vocab::content::crash::current_native_call(), None);
    assert!(mod_api::__private::initialize().is_err());
    // A late platform registration must not acquire attribution retroactively.
    let later = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "late",
        hook: "client_tick_pre",
        part: None,
    });
    assert_eq!(mod_api::content::crash::current_native_call(), None);
    drop(later);
    drop(guard);
    assert_eq!(mod_vocab::content::crash::current_native_call(), None);
}
