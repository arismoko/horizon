use std::path::Path;
use sunrise_hooks::settings::engine;
use sunrise_level::options_file::OptionsFile;
use sunrise_level::sim_settings::SimSettings;
use sunrise_level::tick_mode::ParallelTickMode;
use sunrise_mod_package::mod_list::ModList;

pub(super) struct Workers {
    configured: u32,
    battery: bool,
}

impl Workers {
    pub(super) fn count(&self) -> usize {
        if self.configured > 0 {
            return self.configured as usize;
        }
        let auto = sunrise_threads::jobs::default_threads();
        if self.battery { auto.div_ceil(2) } else { auto }
    }
}

pub(super) fn load(path: &Path) -> (SimSettings, Workers) {
    const EXTRAS: &str = "sunrise-extras.txt";
    const GRAPHICS: &str = "sunrise-graphics.txt";
    let parent = path.parent();
    let extras = parent.map_or_else(String::new, |dir| match engine::read(&dir.join(EXTRAS)) {
        Ok(None) => String::new(),
        Ok(Some(read)) => {
            if read.lossy {
                log::warn!(
                    "Sunrise: {EXTRAS} has bytes that aren't UTF-8, so it won't be saved over"
                );
            }
            read.text
        }
        Err(error) => {
            log::warn!("Sunrise: {EXTRAS} could not be read, so it won't be saved over: {error}");
            String::new()
        }
    });
    let graphics = parent.and_then(|dir| engine::read(&dir.join(GRAPHICS)).ok().flatten());
    engine::load(&mut |file| match file {
        EXTRAS => Some(extras.clone()),
        GRAPHICS => graphics.as_ref().map(|read| read.text.clone()),
        _ => parent.and_then(|dir| {
            engine::read(&dir.join(file))
                .ok()
                .flatten()
                .map(|read| read.text)
        }),
    });

    let mut settings = SimSettings::default();
    let mut workers = Workers {
        configured: 0,
        battery: false,
    };
    for (key, value) in graphics
        .iter()
        .flat_map(|read| read.text.lines())
        .filter_map(setting_pair)
    {
        match key {
            "parallelTick" => settings.parallel_tick = ParallelTickMode::parse(value),
            "workers" => {
                workers.configured = value.parse::<u32>().unwrap_or(workers.configured).min(64)
            }
            "batteryMode" => workers.battery = read_flag(value, workers.battery),
            _ => {}
        }
    }
    for (key, value) in extras.lines().filter_map(setting_pair) {
        match key {
            sunrise_core::no_ai::KEY => settings.no_ai = read_flag(value, settings.no_ai),
            "agentControl" => settings.agent_control = read_flag(value, settings.agent_control),
            _ => {}
        }
    }
    if let Some(dir) = parent {
        settings.mods = ModList::load(&sunrise_level::logging::shared_dir(dir));
    }
    let options = match std::fs::read(path) {
        Ok(bytes) => OptionsFile::parse(&String::from_utf8_lossy(&bytes)),
        Err(_) => OptionsFile::default(),
    };
    apply_options(&mut settings, &options);
    sunrise_level::crash::set_detail("Resource Packs", resource_packs_detail(&options));
    (settings, workers)
}

fn setting_pair(line: &str) -> Option<(&str, &str)> {
    line.split_once(':')
        .map(|(key, value)| (key.trim(), value.trim()))
}

fn read_flag(value: &str, default: bool) -> bool {
    value.parse().unwrap_or(default)
}

fn resource_packs_detail(file: &OptionsFile) -> String {
    let packs = file.list("resourcePacks");
    if packs.is_empty() {
        "vanilla".to_owned()
    } else {
        packs.join(", ")
    }
}

fn apply_options(settings: &mut SimSettings, file: &OptionsFile) {
    settings.render_distance = file.int("renderDistance");
    settings.simulation_distance = file.int("simulationDistance");
    settings.entity_distance = file.double("entityDistanceScaling");
    settings.auto_jump = file.bool("autoJump");
    settings.main_hand_left = file.name("mainHand") == "left";
    settings.chat_visibility = file.int("chatVisibility") as u8;
    settings.operator_items_tab = file.bool("operatorItemsTab");
    settings.reduced_debug_info = file.bool("reducedDebugInfo");
    settings.language = file.string("lang");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_options(contents: Option<&str>) -> (SimSettings, Workers) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("options.txt");
        if let Some(text) = contents {
            std::fs::write(&path, text).unwrap();
        }
        load(&path)
    }

    #[test]
    fn server_options_reader_matches_desktop_options_parser() {
        let (defaults, workers) = load_options(None);
        assert_eq!(defaults, SimSettings::default());
        assert!(workers.count() >= 1);
        assert_eq!(resource_packs_detail(&OptionsFile::default()), "vanilla");

        let (lenient, _) = load_options(Some(
            "version:5023\n\
             renderDistance:/*c*/16.0\n\
             simulationDistance:1.5e1\n\
             entityDistanceScaling:/*c*/2.4\n\
             autoJump:TRUE\n\
             mainHand:'\\u006ceft'\n\
             chatVisibility:5.0\n\
             operatorItemsTab:True\n\
             reducedDebugInfo:tRuE\n\
             lang:fr_fr\n\
             resourcePacks:[\"vanilla\",\"file/x.zip\"]\n",
        ));
        assert_eq!(lenient.render_distance, 16);
        assert_eq!(lenient.simulation_distance, 15);
        assert_eq!(lenient.entity_distance, 2.25);
        assert!(lenient.auto_jump);
        assert!(lenient.main_hand_left);
        assert_eq!(lenient.chat_visibility, 2);
        assert!(lenient.operator_items_tab);
        assert!(lenient.reduced_debug_info);
        assert_eq!(lenient.language, "fr_fr");
        assert_eq!(
            resource_packs_detail(&OptionsFile::parse(
                "version:5023\nresourcePacks:[\"vanilla\",\"file/x.zip\"]\n",
            )),
            "vanilla, file/x.zip"
        );

        let (rejected_nums, _) = load_options(Some(
            "version:5023\nrenderDistance:+20\nsimulationDistance:016\nentityDistanceScaling:+2.0\n",
        ));
        assert_eq!(rejected_nums.render_distance, 12);
        assert_eq!(rejected_nums.simulation_distance, 12);
        assert_eq!(rejected_nums.entity_distance, 1.0);

        let (legacy_lang, _) = load_options(Some("lang:en_US\nrenderDistance:20\n"));
        assert_eq!(legacy_lang.language, "en_us");
        assert_eq!(legacy_lang.render_distance, 20);
        assert_eq!(
            resource_packs_detail(&OptionsFile::parse(
                "version:3200\nresourcePacks:[\"programer_art\"]\n",
            )),
            "programmer_art"
        );

        let (failed_fix, _) = load_options(Some(
            "key_key.jump:key.keyboard.space\nrenderDistance:24\nautoJump:true\nlang:de_de\n",
        ));
        assert_eq!(failed_fix, SimSettings::default());

        // `fov` sits after `entityDistanceScaling` and before the other keys in TABLE order.
        let (aborted, _) = load_options(Some(
            "version:5023\n\
             entityDistanceScaling:2.0\n\
             fov:\"unterminated\n\
             renderDistance:20\n\
             operatorItemsTab:true\n\
             lang:de_de\n",
        ));
        assert_eq!(aborted.entity_distance, 2.0);
        assert_eq!(aborted.render_distance, 12);
        assert!(!aborted.operator_items_tab);
        assert_eq!(aborted.language, "en_us");

        let (bare_cr, _) = load_options(Some(
            "version:5023\rrenderDistance:18\rsimulationDistance:9\rmainHand:\"left\"\rautoJump:true\r",
        ));
        assert_eq!(bare_cr.render_distance, 18);
        assert_eq!(bare_cr.simulation_distance, 9);
        assert!(bare_cr.main_hand_left);
        assert!(bare_cr.auto_jump);
    }
}
