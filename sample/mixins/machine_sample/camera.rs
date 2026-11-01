fn sample_camera(mirrored: bool, entity: &horizon_client::camera::CameraEntity, distance: &mut f32) {
    let mode = std::fs::read_to_string("/box/work/sample-5905/camera-mode").unwrap_or_default();
    let base = *distance;
    match mode.trim() { "double" => *distance *= 2.0, "zero" => *distance = 0.0, "nan" => *distance = f32::NAN, "negative" => *distance = -1.0, "huge" => *distance = 1025.0, _ => {} }
    static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let message = format!("CAMERA_TEST mode={} mirrored={mirrored} base={base} requested={} living={:?} mount={:?}", mode.trim(), *distance, entity.living, entity.mount);
    let mut last = LAST.lock().unwrap();
    if last.as_ref() != Some(&message) { eprintln!("{message}"); *last = Some(message); }
}
