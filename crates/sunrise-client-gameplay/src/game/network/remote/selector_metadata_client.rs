use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn receive_selector_tokens(&mut self, data: &[u8]) -> Result<()> {
        let Some(remote) = self.client.remote.as_mut() else {
            return Ok(());
        };
        if !remote.selector_metadata_supported {
            return Ok(());
        }
        match sunrise_sim::lan::commands::selector_sync::decode(data)? {
            sunrise_sim::lan::commands::selector_sync::Snapshot::Tokens(tokens) => {
                self.common.ui.registries.selector_tokens = tokens;
            }
            sunrise_sim::lan::commands::selector_sync::Snapshot::Unsupported(revision) => {
                remote.selector_metadata_supported = false;
                self.common.ui.registries.selector_tokens.clear();
                log::warn!(
                    "Sunrise: selector metadata revision {revision} is unsupported; tokens disabled for this connection"
                );
            }
        }
        Ok(())
    }
}
