#[cfg(feature = "client")]
use crate::game::ClientGame;
#[cfg(feature = "client")]
use mod_api::{__private::game::CallbackOwner, *};
#[cfg(feature = "client")]
use std::sync::Arc;

#[cfg(feature = "client")]
impl ClientSession for ClientGame<'_> {
    fn client_server(&self) -> Option<ClientServer<'_>> {
        CallbackOwner::call_host(|| self.native_client_server())
    }
    fn client_language(&self) -> Option<Arc<str>> {
        CallbackOwner::call_host(|| self.native_client_language())
    }
    fn client_latency(&self) -> Option<u32> {
        CallbackOwner::call_host(|| self.native_client_latency())
    }
    fn client_tab_list(&self) -> Result<Option<&dyn ClientTabList>, ClientSessionReadError> {
        Ok(CallbackOwner::call_host(|| {
            self.native_session().map(|_| self as &dyn ClientTabList)
        }))
    }

    fn client_waypoints(&self) -> Option<ClientWaypoints> {
        CallbackOwner::call_host(|| self.native_client_waypoints())
    }

    fn client_player_skins(&self) -> Option<ClientPlayerSkins> {
        CallbackOwner::call_host(|| self.native_client_player_skins())
    }
    fn client_fishing_hook(&self) -> Option<ClientFishingHook> {
        CallbackOwner::call_host(|| self.native_client_fishing_hook())
    }
    fn client_titles(&self) -> Result<Option<ClientTitles>, ClientSessionReadError> {
        CallbackOwner::call_host(|| self.native_client_titles())
    }
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        CallbackOwner::call_host(|| self.native_client_frame_stats())
    }
    fn client_boss_bars(&self) -> Result<Option<Vec<ClientBossBarEntry>>, ClientSessionReadError> {
        CallbackOwner::call_host(|| self.native_client_boss_bars())
    }
}
