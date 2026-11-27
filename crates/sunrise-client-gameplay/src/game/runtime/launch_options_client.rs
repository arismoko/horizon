use std::time::Duration;
use sunrise_gameplay_common::game::launch_options::Options;

/// Whether this `Game` is the process's client.
pub fn is_client_start(options: &Options) -> bool {
    options.execution_role.is_client()
}
/// Explicit timeouts apply to every run; only frame-limited runs get a default.
pub(in crate::game) fn time_limit(options: &Options) -> Option<Duration> {
    options
        .timeout
        .or(options.frames.map(|_| Duration::from_secs(120)))
}

pub fn read_client_auth(options: &mut Options) {
    // The launcher's account: online-mode servers need it. A bad file
    // only leaves the game offline.
    if !options.execution_role.is_dedicated_server() {
        let session = if options.developer_single_player {
            Ok(None)
        } else if options.session_stdin {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| e.to_string())
                .and_then(|_| sunrise_client::auth::AuthSession::parse(&text))
                .map(Some)
        } else {
            options
                .session_file
                .take()
                .filter(|p| !p.is_empty())
                .map(|path| sunrise_client::auth::AuthSession::load(path.as_ref()))
                .transpose()
        };
        match session {
            Ok(Some(session)) => {
                options.player_name = Some(session.name.clone());
                sunrise_client::auth::set_current(session);
                // The chat key pair, ready before the first join.
                sunrise_client::auth::prepare_chat_keys(true);
            }
            Ok(None) => {
                options.offline_reason = std::env::var("SUNRISE_OFFLINE_REASON")
                    .ok()
                    .filter(|reason| !reason.is_empty());
            }
            Err(e) => {
                eprintln!("Sunrise: not signed in: {e}");
                options.offline_reason = Some(e);
            }
        }
    }
}
