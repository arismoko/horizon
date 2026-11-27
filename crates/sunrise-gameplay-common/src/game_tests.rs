//! The game-test leaf rules: `TestError` and `RetryOptions`.
use sunrise_game_data::text::Text;

type Pos = (i32, i32, i32);

/// `GameTestAssertException`, `GameTestAssertPosException`,
/// `GameTestTimeoutException` and `UnknownGameTestException`.
#[derive(Clone, Debug)]
pub enum TestError {
    Assert {
        message: Text,
        tick: i32,
    },
    AssertPos {
        message: Text,
        absolute: Pos,
        relative: Pos,
        tick: i32,
    },
    Timeout(Text),
    Unknown(String),
}

pub fn number(v: i32) -> Text {
    Text::literal(v.to_string())
}

impl TestError {
    /// `getDescription`.
    pub fn description(&self) -> Text {
        match self {
            Self::Assert { message, tick } => {
                Text::translate("test.error.tick", &[message.clone(), number(*tick)])
            }
            Self::AssertPos {
                message,
                absolute,
                relative,
                tick,
            } => Text::translate(
                "test.error.position",
                &[
                    message.clone(),
                    number(absolute.0),
                    number(absolute.1),
                    number(absolute.2),
                    number(relative.0),
                    number(relative.1),
                    number(relative.2),
                    number(*tick),
                ],
            ),
            Self::Timeout(message) => message.clone(),
            Self::Unknown(reason) => {
                Text::translate("test.error.unknown", &[Text::literal(reason.clone())])
            }
        }
    }

    /// `getMessage`: the description's characters (a timeout's own message,
    /// an unknown error's reason).
    pub fn message(&self) -> String {
        match self {
            Self::Unknown(reason) => reason.clone(),
            other => other.description().plain(),
        }
    }
}

/// `RetryOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    pub number_of_tries: i32,
    halt_on_failure: bool,
}

impl Retry {
    pub fn new(number_of_tries: i32, halt_on_failure: bool) -> Self {
        Self {
            number_of_tries,
            halt_on_failure,
        }
    }

    /// `noRetries`.
    pub fn none() -> Self {
        Self::new(1, true)
    }

    /// `unlimitedTries`.
    pub fn unlimited(&self) -> bool {
        self.number_of_tries < 1
    }

    /// `hasTriesLeft`.
    pub fn has_tries_left(&self, attempts: i32, successes: i32) -> bool {
        let has_failures = attempts != successes;
        let has_more_attempts = self.unlimited() || attempts < self.number_of_tries;
        has_more_attempts && (!has_failures || !self.halt_on_failure)
    }

    /// `hasRetries`.
    pub fn has_retries(&self) -> bool {
        self.number_of_tries != 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_errors_report_their_reason() {
        let error = TestError::Unknown("no such function".into());
        assert_eq!(error.message(), "no such function");
    }

    #[test]
    fn retry_options_decide_whether_a_test_runs_again() {
        // noRetries: one try, no rerun.
        assert!(!Retry::none().has_retries());
        assert!(!Retry::none().has_tries_left(1, 1));
        // 3 times, not until failed: runs on through failures.
        let three = Retry::new(3, false);
        assert!(three.has_retries());
        assert!(three.has_tries_left(1, 0));
        assert!(three.has_tries_left(2, 2));
        assert!(!three.has_tries_left(3, 3));
        // Until failed: stops at the first failure, and 0 means unlimited.
        let until = Retry::new(0, true);
        assert!(until.unlimited());
        assert!(until.has_tries_left(50, 50));
        assert!(!until.has_tries_left(50, 49));
    }
}
