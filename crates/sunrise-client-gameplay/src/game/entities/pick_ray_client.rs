use sunrise_gameplay_common::game::pick_ray::PickAccumulator;
use sunrise_gameplay_common::game::pick_ray::PickCandidate;

pub(in crate::game) trait PickRecordingExt: Sized {
    fn nearest_sq(&self) -> f64;
    fn recording_with(max_distance_sq: f64, candidates: Vec<PickCandidate>) -> Self;
    fn take_candidates(&mut self) -> Vec<PickCandidate>;
}
impl PickRecordingExt for PickAccumulator {
    fn nearest_sq(&self) -> f64 {
        self.nearest_sq
    }
    fn recording_with(max_distance_sq: f64, mut candidates: Vec<PickCandidate>) -> Self {
        candidates.clear();
        Self {
            candidates: Some(candidates),
            ..Self::new(max_distance_sq)
        }
    }
    fn take_candidates(&mut self) -> Vec<PickCandidate> {
        self.candidates.take().unwrap_or_default()
    }
}
