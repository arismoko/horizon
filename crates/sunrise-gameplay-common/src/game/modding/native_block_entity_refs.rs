#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_core::nbt::{Compound, Tag};

    pub fn compound(snbt: &str) -> Compound {
        match sunrise_core::nbt::parse_snbt(snbt).unwrap().0 {
            Tag::Compound(c) => c,
            _ => panic!("not a compound"),
        }
    }
}
