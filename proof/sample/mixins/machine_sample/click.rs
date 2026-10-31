fn sample_click(block: &str, at: [f32; 2], right: bool, button: &mut Option<i32>) {
    if block == super::machine_sample::MACHINE
        && !right
        && (66.0..110.0).contains(&at[0])
        && (55.0..71.0).contains(&at[1])
    {
        *button = Some(super::machine_sample::DEPOSIT);
    }
}
