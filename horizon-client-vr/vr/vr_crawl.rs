//! Vivecraft's `CrawlTracker`, ported from VivecraftMod (LGPL-3.0,
//! `client_vr/gameplay/trackers/CrawlTracker.java`): lie down in real life to crawl. Vivecraft
//! needs its server for this (`SERVER_ALLOWS_CRAWLING`); here the integrated server runs in this
//! process and reads the crawl from `vr_crawl_host`, so it crawls in single player and when hosting
//! a LAN world; on a server with Vivecraft's plugin it is told (`CrawlPayloadC2S`, `vr_net`).
use super::Game;
use super::vr_player::vr_player;
use super::vr_settings::vr_settings;

/// `crawlThreshold`: the head this low (meters, at world scale 1) crawls.
const CRAWL_THRESHOLD: f32 = 0.82;

/// The crawling owner's UUID, for this process's integrated server (its guest body crawls too).
static HOST_CRAWL: std::sync::Mutex<Option<u128>> = std::sync::Mutex::new(None);

/// Tracking stopped or went seated: the server's body stands again.
pub(crate) fn vr_crawl_stop() {
    *HOST_CRAWL.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub(crate) fn vr_crawl_host() -> Option<u128> {
    *HOST_CRAWL.lock().unwrap_or_else(|e| e.into_inner())
}

impl Game<'_> {
    pub(super) fn vr_crawl_tracker(&mut self) {
        let active = vr_settings().crawl
            && !vr_settings().seated
            && self.serving.is_none()
            && (self.has_singleplayer_server() || super::vr_net::server_allows_crawling())
            && self.screen != super::Screen::Dead
            && self.sleeping.is_none()
            && self.riding.is_none()
            && self.player.as_ref().is_some_and(|p| !p.spectator);
        let crawling = active
            && vr_player().as_ref().is_some_and(|vr| {
                vr.trackers.hmd_pivot_history.average_position(0.2).y * vr.world_scale + 0.1 < CRAWL_THRESHOLD
            });
        if let Some(p) = self.player.as_mut() {
            p.vr_crawl = crawling;
        }
        *HOST_CRAWL.lock().unwrap_or_else(|e| e.into_inner()) = crawling.then(|| self.own_uuid());
    }
}
