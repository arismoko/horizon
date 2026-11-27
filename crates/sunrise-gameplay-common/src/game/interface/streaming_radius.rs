/// Mesh streaming keeps the extra column it needs, separately from render and cache radii.
pub fn streaming_radius_for(client_render_distance: i32, server_view_distance: Option<i32>) -> i32 {
    match server_view_distance {
        Some(server) if server > 0 => client_render_distance.min(server.saturating_add(1)),
        _ => client_render_distance,
    }
}
