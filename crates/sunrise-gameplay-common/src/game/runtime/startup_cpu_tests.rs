use super::*;

#[test]
fn preflight_names_missing_damaged_or_wrong_profile_game_files() {
    use sunrise_core::prepared_profile::{MANIFEST, Profile, SCHEMA, STAMP};
    let root = std::env::temp_dir().join(format!("sunrise-unprepared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let missing = preflight_resources(&root, None, false).unwrap_err();
    assert!(missing.contains(MANIFEST), "{missing}");
    assert!(missing.contains("sunrise-unprepared-"), "{missing}");
    assert!(missing.contains("sunrise-prepare --prepare"), "{missing}");
    assert!(!missing.contains("launcher"), "{missing}");
    let get = sunrise_hooks::GetTheGame {
        address: "https://example.org/get",
        title: "",
        paragraphs: &[],
        button: "",
    };
    let missing = preflight_resources(&root, Some(get), false).unwrap_err();
    assert!(
        missing.contains("launcher: download it at https://example.org/get"),
        "{missing}"
    );
    std::fs::create_dir_all(&root).unwrap();
    let mut manifest =
        serde_json::json!({"schema": SCHEMA, "minecraft": sunrise_core::version::VERSION_ID});
    std::fs::write(root.join(MANIFEST), manifest.to_string()).unwrap();
    let missing_stamp = preflight_resources(&root, None, false).unwrap_err();
    assert!(
        missing_stamp.contains("output version unknown"),
        "{missing_stamp}"
    );
    std::fs::write(root.join(STAMP), r#"{"output_version":7}"#).unwrap();
    let stale = preflight_resources(&root, None, false).unwrap_err();
    assert!(
        stale.contains("output version 7")
            && stale.contains("sunrise-prepare --prepare --server-jar"),
        "{stale}"
    );
    let stamp = serde_json::json!({"output_version": Profile::Full.output_version()});
    std::fs::write(root.join(STAMP), stamp.to_string()).unwrap();
    assert_eq!(preflight_resources(&root, None, false), Ok(Profile::Full));

    std::fs::write(root.join("gameplay-facts.json"), "").unwrap();
    let damaged = preflight_resources(&root, None, false).unwrap_err();
    assert!(
        damaged.contains("gameplay-facts.json is empty"),
        "{damaged}"
    );
    manifest["profile"] = "server".into();
    std::fs::write(root.join(MANIFEST), manifest.to_string()).unwrap();
    let server_only = preflight_resources(&root, None, false).unwrap_err();
    assert!(
        server_only.contains("server-only") && server_only.contains("--output FULL_DIR"),
        "{server_only}"
    );
    std::fs::write(root.join(MANIFEST), " \n").unwrap();
    assert!(
        preflight_resources(&root, None, false)
            .unwrap_err()
            .contains(MANIFEST)
    );
    std::fs::remove_dir_all(&root).unwrap();
}
