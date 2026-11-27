#[cfg(test)]
pub(in crate::game) mod tests {
    use std::time::Duration;
    use sunrise_gameplay_common::game::launch_options::Options;

    fn limit(args: &[&str]) -> Option<Duration> {
        crate::game::client_side::launch_options_client::time_limit(
            &Options::parse(args.iter().map(|arg| arg.to_string()))
                .unwrap()
                .unwrap(),
        )
    }

    /// Upstream's regression: a control-socket run must not outlive its explicit deadline.
    #[test]
    fn timeout_seconds_limits_every_run() {
        let secs = |n| Some(Duration::from_secs(n));
        assert_eq!(limit(&[]), None);
        assert_eq!(limit(&["--control-socket", "c.sock"]), None);
        assert_eq!(
            limit(&["--control-socket", "c.sock", "--timeout-seconds", "5"]),
            secs(5)
        );
        assert_eq!(limit(&["--timeout-seconds", "600"]), secs(600));
        assert_eq!(limit(&["--frames", "10"]), secs(120));
        assert_eq!(limit(&["--world-frames", "10"]), secs(120));
        assert_eq!(
            limit(&["--frames", "10", "--timeout-seconds", "30"]),
            secs(30)
        );
    }

    /// #5304: a world `--world` creates without `--seed` gets a random seed, not seed 0;
    /// a given seed reads as a number or, as `WorldOptions.parseSeed`, as text's hash.
    #[test]
    fn world_seed_is_left_to_the_new_world_unless_given() {
        let seed = |args: &[&str]| {
            Options::parse(args.iter().map(|arg| arg.to_string()))
                .unwrap()
                .unwrap()
                .seed
        };
        assert_eq!(seed(&["--world", "W"]), None);
        assert_eq!(seed(&["--world", "W", "--seed", "0"]), Some(0));
        assert_eq!(seed(&["--world", "W", "--seed", "-12345"]), Some(-12345));
        // "abc".hashCode() in Java.
        assert_eq!(seed(&["--world", "W", "--seed", "abc"]), Some(96354));
    }
}
