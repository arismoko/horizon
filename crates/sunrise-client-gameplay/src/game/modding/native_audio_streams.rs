//! The game opens client mods' audio streams on the mixer's separate pool and
//! places the entity-following ones each frame. Also exposes client microphone capture.
use crate::game::ClientGame;
use mod_api::{
    AudioRefusal, AudioStream, AudioStreamRef, MicDevice, MicRef, MicRefusal, MicSpec,
    StreamPlacement, StreamSpec,
};

/// The mod whose call this is: the host marks it when it enters the native callback.
/// A call from no mod opens nothing.
fn calling_mod() -> Result<&'static str, AudioRefusal> {
    mod_api::content::crash::current_native_mod().ok_or(AudioRefusal::Unavailable)
}

impl ClientGame<'_> {
    /// Opens a stream the mixer pulls from on its worker.
    pub(in crate::game) fn open_audio_stream(
        &self,
        spec: &StreamSpec,
        source: Box<dyn AudioStream>,
    ) -> Result<AudioStreamRef, AudioRefusal> {
        if !self.common.native_window() {
            return Err(AudioRefusal::OnServer);
        }
        let entity_pos = match spec.placement {
            StreamPlacement::Entity(entity) => self.entity_position(entity),
            StreamPlacement::Flat | StreamPlacement::At(_) => None,
        };
        self.client
            .audio
            .0
            .open_stream(calling_mod()?, spec, entity_pos, source)
            .map(AudioStreamRef::new)
    }
    /// Places every entity-following stream at its entity, or stops it where the client
    /// tracks no such entity. Once a frame, beside the listener's own update.
    pub fn follow_audio_stream_entities(&self) {
        self.client
            .audio
            .0
            .follow_stream_entities(|entity| self.entity_position(entity));
    }

    /// Lists the client's audio input devices for
    /// [`mod_api::prelude::AudioClientHost::microphone_devices`].
    pub(in crate::game) fn microphone_devices(&self) -> Result<Vec<MicDevice>, MicRefusal> {
        if !self.common.native_window() {
            return Err(MicRefusal::OnServer);
        }
        self.client.audio.0.microphone_devices()
    }

    /// Opens a client microphone capture handle for
    /// [`mod_api::prelude::AudioClientHost::open_microphone`].
    pub(in crate::game) fn open_microphone(&self, spec: &MicSpec) -> Result<MicRef, MicRefusal> {
        if !self.common.native_window() {
            return Err(MicRefusal::OnServer);
        }
        self.client.audio.0.open_microphone(spec).map(MicRef::new)
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::content::crash::ModCall;
    use mod_platform::content::crash::enter_native_mod;

    #[test]
    fn each_calling_mod_is_counted_on_its_own_and_no_mod_opens_nothing() {
        assert_eq!(calling_mod(), Err(AudioRefusal::Unavailable));
        let call = |id| ModCall {
            id,
            hook: "client_tick",
            part: None,
        };
        let _a = enter_native_mod(call("mod_a"));
        assert_eq!(calling_mod(), Ok("mod_a"));
        let _b = enter_native_mod(call("mod_b"));
        assert_eq!(calling_mod(), Ok("mod_b"));
    }
}
