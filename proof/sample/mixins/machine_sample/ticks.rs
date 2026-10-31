fn sample_ticks(block: &str, ticks: &mut bool) {
    if block == super::machine_sample::MACHINE {
        *ticks = true;
    }
}
