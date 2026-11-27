#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    pub fn assert_near(got: [f64; 3], want: [f64; 3]) {
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1.0e-4, "{got:?} is not {want:?}");
        }
    }
}
