use std::sync::mpsc::Sender;

use crate::control::Request;

pub use sunrise_sim::benchmark::{
    SCENES, SEED, SWITCHES, Scenario, Setup, Switch, WORLD, parse_set,
};

/// Starts the driver; the game quits when it is done, and the report's path is printed.
pub fn start(requests: Sender<Request>, setup: Setup) {
    sunrise_sim::benchmark::start(requests, setup, crate::telemetry::keep_benchmark);
}
