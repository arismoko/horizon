use super::*;

/// Capture a standalone engine style through the same text boundary.
pub fn style_to_api(style: &engine::Style) -> Result<TextStyle, TextFormError> {
    let span = engine::Span::new("", style.clone());
    Ok(std::mem::take(&mut capture(Source::Span(&span))?.style))
}

/// Capture a standalone engine style in memory without failing on unrepresentable opaque metadata.
pub fn style_to_api_inprocess(style: &engine::Style) -> TextStyle {
    let span = engine::Span::new("", style.clone());
    std::mem::take(
        &mut capture_with(Source::Span(&span), true)
            .expect("lenient capture is infallible")
            .style,
    )
}
