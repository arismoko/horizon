#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    type Tank = (Option<String>, i32);

    pub fn tanks(list: &[(Option<&str>, i32)]) -> Vec<Tank> {
        list.iter()
            .map(|&(fluid, amount)| (fluid.map(str::to_owned), amount))
            .collect()
    }
}
