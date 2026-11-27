fn main() {
    let table = r#"
static FIXTURES: std::sync::LazyLock<Vec<(&'static str, fn())>> = std::sync::LazyLock::new(|| {
    let mut rows = Vec::new();
    rows.extend_from_slice(sunrise_gameplay_common::parity_fixtures());
    rows.extend_from_slice(sunrise_gameplay::parity_fixtures());
    #[cfg(feature = "client-defaults")]
    {
        rows.extend_from_slice(sunrise_client_gameplay::parity_fixtures());
        rows.extend_from_slice(sunrise_client::parity_fixtures());
    }
    rows.sort_unstable_by_key(|row| row.0);
    for pair in rows.windows(2) {
        assert_ne!(pair[0].0, pair[1].0, "duplicate parity fixture");
    }
    rows
});
"#;
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(path.join("parity_fixtures.rs"), table).expect("write parity dispatcher table");
}
