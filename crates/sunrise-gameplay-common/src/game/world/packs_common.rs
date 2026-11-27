use mod_api::__private::game::*;
use mod_api::{__private::reload::ReloadFiles, FileToIdConverter, Identifier};
use std::path::PathBuf;
use sunrise_level::datapack::DataPack;

/// Where overlay roots are built (beside options.txt).
pub fn cache_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join(".sunrise-cache/resourcepacks")
}

/// Reads one converter's directory from `packs`, later packs winning, and
/// collects the converted files: the shared read-and-log loop of both sides'
/// reloads. `top` is `"data"` or `"assets"`.
pub fn reload_files(
    top: &str,
    packs: &[&DataPack],
    listener: &Identifier,
    converter: &FileToIdConverter,
) -> ReloadFiles {
    let mut selected = std::collections::BTreeMap::new();
    for pack in packs {
        for file in pack.list_under(top, &[converter.directory()]) {
            if let Some(id) = file
                .strip_prefix(&format!("{top}/"))
                .and_then(|path| converter.file_to_id(path))
            {
                selected.insert(id, (*pack, file));
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    for (id, (pack, path)) in selected {
        // A file that vanishes or fails to read between listing and reading is one bad
        // entry, as vanilla's `scanDirectory` treats a parse failure: log it, skip it.
        match pack.read(&path) {
            Some(bytes) => {
                files.insert(id, bytes);
            }
            None => log::error!(
                "Couldn't read file '{id}' from '{}/{}' for listener {}",
                pack.id,
                path,
                listener
            ),
        }
    }
    ReloadFiles::new(files)
}
