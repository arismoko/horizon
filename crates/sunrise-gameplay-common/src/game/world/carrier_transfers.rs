#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    pub fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    /// A fresh throwaway world root with `data/` and a readable
    /// `level.dat`, named after the test.
    /// The region directory's files with their bytes, for before/after
    /// comparisons that no save wrote anything.
    pub fn region_files(region: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(region) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                files.push((name, std::fs::read(entry.path()).unwrap_or_default()));
            }
        }
        files.sort();
        files
    }
    pub fn transfer_root(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("sunrise-carrier-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        write_level_dat(&root);
        root
    }
    pub fn write_level_dat(root: &std::path::Path) {
        // A current DataVersion inside Data keeps the world-gen reader
        // out of the data fixers, whose tables the prepared test
        // resources do not carry.
        let mut data = sunrise_core::nbt::Compound::new();
        data.insert(
            "DataVersion".into(),
            sunrise_core::nbt::Tag::Int(sunrise_world::DATA_VERSION),
        );
        sunrise_sim::save::write_dat(
            &root.join("level.dat"),
            sunrise_core::nbt::Tag::compound([("Data", sunrise_core::nbt::Tag::Compound(data))]),
        )
        .unwrap();
    }
}
