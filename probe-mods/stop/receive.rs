fn stop_message_received(channel: &str, data: &[u8]) {
    if channel == "stop_probe:stop" && data == b"farewell" {
        if let Ok(path) = std::env::var("STOP_RECEIVED") {
            let _ = std::fs::write(path, "farewell received");
        }
    }
}
