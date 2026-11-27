#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    /// The UUID the transfer tests give their attacker, as `summon` takes it.
    pub const ZOMBIE_UUID: [i32; 4] = [7, 8, 9, 10];
    pub const ZOMBIE_NBT: &str = "{NoAI:1b,UUID:[I;7,8,9,10]}";
}
