fn sample_key_release(screen: &NativeScreenView<'_>, key: &str, canceled: &mut bool) {
    *canceled = key == "key.keyboard.a";
    eprintln!("RELEASE_TEST screen={} key={key} canceled={canceled}", screen.id);
}
