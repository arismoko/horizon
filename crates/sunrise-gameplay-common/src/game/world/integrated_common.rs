pub trait IntegratedCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `Minecraft.hasSingleplayerServer`: a world of this window's own, run on its integrated server.
    fn has_singleplayer_server(&self) -> bool {
        self.side_has_singleplayer_server()
    }

    /// `Minecraft.isMultiplayerServer`: on another's server, or a world of one's own open to LAN.
    fn is_multiplayer_server(&self) -> bool {
        !self.has_singleplayer_server() || self.is_published()
    }

    /// `IntegratedServer.isPublished`: the world is open to the network.
    fn is_published(&self) -> bool {
        self.side_is_published()
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> IntegratedCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::presentation::PresentationSideGameExt as _;
