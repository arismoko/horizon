//! Opt-in test observations after successful NETWORK preparation, before publication.
use std::sync::Mutex;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub work: usize,
    pub replies: usize,
}
static WATCH: Mutex<
    Option<(
        String,
        Counts,
        Vec<std::sync::Arc<super::endpoint::Observation>>,
    )>,
> = Mutex::new(None);
pub struct Watch;
impl Watch {
    pub fn begin(channel: &str) -> Self {
        let mut watch = WATCH.lock().unwrap();
        assert!(watch.is_none());
        *watch = Some((channel.into(), Counts::default(), Vec::new()));
        Self
    }
    /// Live observations, not an atomic cross-queue snapshot. Observations retain
    /// only weak ownership handles, never connection owners or payload credit leases.
    pub fn diagnostics(&self) -> Vec<String> {
        let endpoints = match WATCH.try_lock() {
            Ok(watch) => watch.as_ref().map(|w| w.2.clone()).unwrap_or_default(),
            Err(_) => return vec!["watch busy/poisoned".into()],
        };
        endpoints
            .iter()
            .map(|observation| observation.diagnostics())
            .collect()
    }
    pub fn snapshot(&self) -> Counts {
        WATCH.lock().unwrap().as_ref().unwrap().1
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        *WATCH.lock().unwrap() = None;
    }
}
pub(super) fn prepared(channel: &str, work: bool) {
    if let Some((watched, counts, _)) = WATCH.lock().unwrap().as_mut()
        && watched == channel
    {
        if work {
            counts.work += 1;
        } else {
            counts.replies += 1;
        }
    }
}

pub(super) fn capture(channel: &str, endpoint: &super::Endpoint) {
    if let Some((watched, _, endpoints)) = WATCH.lock().unwrap().as_mut()
        && watched == channel
        && !endpoints.iter().any(|e| e.same_target(endpoint))
    {
        endpoints.push(std::sync::Arc::new(endpoint.observation()));
    }
}
