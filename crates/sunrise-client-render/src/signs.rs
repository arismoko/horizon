//! Sign block entities (`SignBlockEntity`, `SignText`): re-exported from `sunrise_level::signs_data`.
pub use sunrise_level::signs_data::*;

#[cfg(test)]
mod tests {
    #[test]
    fn a_mods_sign_like_name_does_not_make_it_a_sign() {
        assert!(crate::containers::BlockEntity::for_block("ruby:road_sign").is_none());
    }
}
