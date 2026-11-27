pub(in crate::game) use sunrise_gameplay::cauldron_contents::{
    CauldronContent, CauldronContents, EMPTY_CAULDRON, current,
};

#[cfg(any(test, feature = "test-support"))]
pub fn without_built_table(test: impl FnOnce()) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            sunrise_gameplay::cauldron_contents::swap_built(self.0);
        }
    }
    let _restore = Restore(sunrise_gameplay::cauldron_contents::swap_built(false));
    test();
}
