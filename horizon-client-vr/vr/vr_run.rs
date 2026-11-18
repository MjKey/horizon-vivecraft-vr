//! Vivecraft's `RunTracker` (`vrFreeMoveMode = RUN_IN_PLACE`), ported from VivecraftMod
//! (LGPL-3.0, `client_vr/gameplay/trackers/RunTracker.java`): swing both arms as when running to
//! walk where they point.
use super::Game;
use super::vr_player::{vr_player, yaw_of};
use super::vr_settings::{FreeMove, vr_settings};
use std::sync::Mutex;

/// The run's yaw (degrees) and speed (0, 1 or 1.3), this tick.
static RUN: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

/// Drops a run-in-place input when VR stops, so it cannot feed a normal pad later.
pub(crate) fn vr_run_reset() {
    *RUN.lock().unwrap_or_else(|e| e.into_inner()) = (0.0, 0.0);
}

/// The yaw the run goes, while running.
pub(crate) fn vr_run_yaw() -> Option<f32> {
    let (yaw, speed) = *RUN.lock().unwrap_or_else(|e| e.into_inner());
    (speed > 0.0).then_some(yaw)
}

/// The run as the left stick's forward push (the walk input), while run in place is on.
pub(crate) fn vr_run_stick() -> Option<[f32; 2]> {
    (horizon_vr::poses().is_some() && vr_settings().free_move == FreeMove::RunInPlace && !vr_settings().seated)
        .then(|| [0.0, RUN.lock().unwrap_or_else(|e| e.into_inner()).1.min(1.0)])
}

impl Game<'_> {
    /// `RunTracker.isActive` and `activeProcess`.
    pub(super) fn vr_run_tracker(&mut self) {
        let active = vr_settings().free_move == FreeMove::RunInPlace
            && !vr_settings().seated
            && self.screen != super::Screen::Dead
            && self.player.as_ref().is_some_and(|p| {
                !(!p.ground.on_ground && p.fluids.was_touching_water)
                    && !p.on_climbable
            });
        let mut run = RUN.lock().unwrap_or_else(|e| e.into_inner());
        let guard = vr_player();
        let Some(vr) = guard.as_ref().filter(|v| active && !v.trackers.bow.is_notched()) else {
            run.1 = 0.0;
            return;
        };
        let Some(data) = vr.pre else {
            run.1 = 0.0;
            return;
        };
        let h = &vr.trackers.controller_history;
        let (c0, c1) = (h[0].average_speed(0.33), h[1].average_speed(0.33));
        let threshold = if run.1 > 0.0 { 0.1 } else { 0.6 };
        if (c0 < threshold && c1 < threshold) || (c0 - c1).abs() > 0.5 {
            run.1 = 0.0;
            return;
        }
        let v = (data.controller_dir(0) + data.controller_dir(1)) * 0.5;
        run.0 = yaw_of(v);
        // As Vivecraft: any real swing is full speed.
        let speed = (c0 + c1) * 0.5 * 1.3;
        run.1 = if speed > 0.1 { 1.0 } else { speed };
    }
}
