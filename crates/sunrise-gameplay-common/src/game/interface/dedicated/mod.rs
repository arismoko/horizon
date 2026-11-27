#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    pub const LOGIN: &str = "Player[/127.0.0.1:0] logged in with entity id 2 at (0.5, 115.0, 0.5)";
    pub const WARNING: &str = "Authentication warning";
    pub const READY: &str = "sunrise-console-log-ready\n";
    pub const CHILD_HOME: &str = "SUNRISE_TEST_CONSOLE_LOG_HOME";
    pub fn assert_log_lines(text: &str) {
        let expected = [
            format!("] [Server thread/INFO]: {LOGIN}"),
            format!("] [User Authenticator #1/WARN]: {WARNING}"),
        ];
        assert_eq!(text.lines().count(), expected.len());
        for (line, suffix) in text.lines().zip(expected) {
            assert!(line.starts_with('['));
            assert!(chrono::NaiveTime::parse_from_str(&line[1..9], "%H:%M:%S").is_ok());
            assert_eq!(&line[9..], suffix);
        }
    }
}
