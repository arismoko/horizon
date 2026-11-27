/// The server side's capture state: the integrated server thread's history/capture and guests' remote requests.
#[derive(Default)]
pub(in crate::game) struct ServerProfile {
    /// The server thread's health history and its side of a capture; set on its own Game only.
    pub(in crate::game) server: Option<super::integrated::capture::ServerSide>,
    /// The guests' requests to this server and the capture they ask of it (perf_remote.rs).
    pub(in crate::game) served: super::perf_remote::Served,
}

#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::profile_capture_common::shown_in;

    #[test]
    fn a_capture_under_the_home_is_shown_with_slashes() {
        let home = std::env::temp_dir().join("sunrise-home");
        let inside = home.join("reports").join("profile-20261113-190003Z.hzperf");
        assert_eq!(
            shown_in(&inside, &home),
            "reports/profile-20261113-190003Z.hzperf"
        );
        let elsewhere = std::env::temp_dir().join("captures").join("a.hzperf");
        assert_eq!(shown_in(&elsewhere, &home), elsewhere.display().to_string());
    }
}
