#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use mod_api::{ClockAdjustment, Identifier};

    pub const NOW: i64 = 13_000;
    fn id(s: &str) -> Identifier {
        Identifier::parse(s).unwrap()
    }
    pub fn marker(s: &str) -> ClockAdjustment {
        ClockAdjustment::Marker(id(s))
    }
}
