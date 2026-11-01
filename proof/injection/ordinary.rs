fn ordinary() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("INJECT_PROOF ordinary HEAD fired"));
}
