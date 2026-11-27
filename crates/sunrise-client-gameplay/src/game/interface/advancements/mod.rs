#[cfg(test)]
pub(crate) mod tests {

    use sunrise_sim::advancements::Frame;

    use crate::game::client_side::advancement_toast::toast_label;

    #[test]
    fn toast_labels_are_yellow_or_pink() {
        // The batch takes colours as `rgba`: 0xFFFF_FF00 there is cyan (#5850).
        for frame in [Frame::Task, Frame::Goal] {
            assert_eq!(toast_label(frame).1.to_le_bytes(), [255, 255, 0, 255]);
        }
        assert_eq!(
            toast_label(Frame::Challenge).1.to_le_bytes(),
            [255, 136, 255, 255]
        );
    }
}
