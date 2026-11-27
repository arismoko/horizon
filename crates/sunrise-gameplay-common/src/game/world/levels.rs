#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_sim::save::SaveDir;

    pub fn queue_unencodable_entity_record(save: &SaveDir) {
        use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
        let invalid =
            (0..=DISK_WRITE_NESTING_GUARD).fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
        save.save_entities_later(vec![((2, 0), vec![invalid])])
            .unwrap();
    }
    pub static LOGGED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
}
