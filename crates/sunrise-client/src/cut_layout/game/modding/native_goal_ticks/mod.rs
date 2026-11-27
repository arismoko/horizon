#![cfg(test)]
#[cfg(test)]
mod interception_tests;
#[cfg(test)]
pub(crate) mod live_read_tests;
#[cfg(test)]
mod redirect_tests;

#[cfg(test)]
#[path = "native_goal_ticks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_behavior_tests.rs"]
mod behavior_tests;
