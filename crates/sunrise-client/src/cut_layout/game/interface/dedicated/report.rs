#![cfg(test)]
#[cfg(feature = "client")]
use std::path::Path;
#[cfg(feature = "client")]
use std::path::PathBuf;
#[cfg(feature = "client")]
use sunrise_perf_format::v1::StopReason;
#[cfg(test)]
mod tests {
    #[cfg(feature = "client")]
    use super::*;
    #[cfg(feature = "client")]
    use std::time::Instant;
    #[cfg(feature = "client")]
    use sunrise_gameplay::dedicated::report::Recording;
    #[cfg(feature = "client")]
    use sunrise_gameplay::game::server_side::dedicated::report::Plan;
    #[cfg(feature = "client")]
    use sunrise_level::spans;
    #[cfg(feature = "client")]
    use sunrise_perf_format::v1::Trigger;
    #[cfg(feature = "client")]
    use sunrise_sim::server_perf::SamplerOptions;

    #[cfg(feature = "client")]
    fn server_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        root: &Path,
    ) -> sunrise_gameplay::game::ServerGame<'a> {
        use {
            crate::game::load_resources,
            crate::settings::Settings,
            sunrise_gameplay_common::game::{Options, Progress},
        };
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            Vec::new(),
            None,
            root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let resources = &*resources.insert(loaded.resources);
        sunrise_gameplay::game::ServerGame::new(
            resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap()
    }

    /// A server that holds a seed, a world name, a player's name, a named mob and a sign writes a capture with none of them.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_capture_of_a_server_with_names_text_and_a_seed_holds_none_of_them() {
        use std::io::Read;
        use sunrise_level::containers::BlockEntity;
        use {sunrise_level::signs_data::SignData, sunrise_level::signs_data::SignText};
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let mut resources = None;
        let mut game = server_game(&mut resources, &root);
        game.level.name = "SecretWorldName".into();
        game.level.seed = 8_675_309;
        game.options.player_name = Some("SteveSecret".into());
        let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        let at = sunrise_core::geometry::Vec3::new(0.5, 4.0, 0.5);
        let mut cow = sunrise_sim::mobs::Mob::new(1, kind, at, 0.0);
        cow.custom_name = Some("Zorblax".into());
        game.mobs.push(cow);
        let sign = SignData {
            front: SignText {
                lines: [
                    "SecretSignText".into(),
                    String::new(),
                    String::new(),
                    String::new(),
                ],
                ..SignText::default()
            },
            ..SignData::default()
        };
        game.world
            .block_entities
            .insert((1, 64, 1), BlockEntity::Sign(sign));

        let dir = std::env::temp_dir().join(format!("hz-privacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut perf = sunrise_sim::server_perf::ServerPerf::new(0.0, 250.0);
        let sunrise = sunrise_sim::server_perf::sunrise_options(0);
        let plan = Plan {
            seconds: 2,
            samples: true,
            timeline: false,
            file: None,
            trigger: Trigger::Console,
            sampler: SamplerOptions::default(),
        };
        let options = game.capture_options(&plan, ((10, 8), 20, 256), sunrise);
        perf.start_capture(0.0, &dir, options).unwrap();
        for i in 0..40 {
            let detail = sunrise_sim::server_perf::TickDetail {
                cpu_ms: None,
                spans: vec![spans::Span {
                    name: "tick",
                    ms: 10.0,
                    cpu_ms: 9.0,
                    depth: 0,
                }],
            };
            let t = f64::from(i) / 20.0;
            perf.tick(
                t,
                10.0,
                Some(detail),
                || game.perf_world_counts(),
                || game.wire_bytes(),
            );
        }
        let recording = Recording {
            seconds: 2,
            all_threads: false,
            started: Instant::now(),
            ticks: 40,
            sampled_ticks: 10,
        };
        let end = game.capture_end(&recording);
        let saved = perf
            .stop_capture(2.0, StopReason::Normal, end)
            .unwrap()
            .unwrap();

        let gz = std::fs::read(&saved.path).unwrap();
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_end(&mut bytes)
            .unwrap();
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        for secret in [
            "SecretWorldName",
            "SteveSecret",
            "Zorblax",
            "SecretSignText",
            "8675309",
        ] {
            assert!(!has(secret.as_bytes()), "the capture holds {secret}");
        }
        // The entities and block entities are there, by type only.
        let c = sunrise_perf::read_capture(&gz).unwrap();
        let text = |id: u32| c.strings.iter().find(|s| s.id == id).unwrap().text.as_str();
        let stop = c.world.last().unwrap();
        let types = |counts: &[sunrise_perf_format::v1::TypeCount]| {
            counts
                .iter()
                .map(|t| (text(t.r#type), t.count))
                .collect::<Vec<_>>()
        };
        assert_eq!(text(stop.dims[0].dimension), "minecraft:overworld");
        assert_eq!(types(&stop.dims[0].entities), [("minecraft:cow", 1)]);
        assert_eq!(types(&stop.dims[0].block_entities).len(), 1);
        // Symbols and source filenames are free text by design; the secret scan above
        // still covers them. Every other string is a registry ID, span/job name or thread.
        let symbols: std::collections::HashSet<u32> = c
            .samples
            .iter()
            .filter_map(|profile| profile.functions.as_ref())
            .flat_map(|functions| {
                functions
                    .name
                    .iter()
                    .chain(&functions.file)
                    .chain(&functions.crate_)
                    .copied()
            })
            .collect();
        for s in c.strings.iter().filter(|s| !symbols.contains(&s.id)) {
            let plain = s
                .text
                .chars()
                .all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '_' | '.' | ':' | '/' | '-'));
            assert!(plain || s.text == "Server thread", "{}", s.text);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
