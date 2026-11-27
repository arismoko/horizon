fn observe_commands() {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD Hub::send_permission_level (sunrise_sim)"));
}
