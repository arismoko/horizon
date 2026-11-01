fn probe() {
    if let Ok(path) = std::env::var("HORIZON_FRAGMENT_PROOF") {
        let _ = std::fs::write(path, "fragment hook fired");
    }
}
