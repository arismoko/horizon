fn self_move() {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD motion::MotionWorld::move_self (sunrise_sim)"));
}
