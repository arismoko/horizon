use horizon_engine_slots::{POINTS, Refusal, install_seal};

#[test]
fn published_points_refuse_a_late_provider() {
    assert_eq!(POINTS.publish([true]), Ok(()));
    assert_eq!(install_seal(|| {}), Err(Refusal::AlreadyPublished));
    assert!(POINTS.active(0));
}
