//! Vivecraft's handheld camera, ported from VivecraftMod (LGPL-3.0,
//! https://github.com/Vivecraft/VivecraftMod, `client_vr/gameplay/trackers/CameraTracker.java`):
//! toggled from the radial menu, it appears in front of the main hand and stays in the world;
//! grab it with the right grip to move it, pull the right trigger at it to take a photo. Its
//! picture shows on its back (`horizon_vr` draws the view and the screen).
use super::vr_player::VrData;
use super::Game;
use glam::{Quat, Vec3};
use std::sync::Mutex;

/// How close the main hand must be to grab or shoot (meters).
const REACH: f64 = 0.15;

struct Camera {
    pos: [f64; 3],
    rot: Quat,
    /// While grabbed: the camera in the hand's frame.
    grab: Option<(Vec3, Quat)>,
    near: bool,
    buttons: [bool; 2],
}

static CAMERA: Mutex<Option<Camera>> = Mutex::new(None);

fn camera<T>(f: impl FnOnce(&mut Option<Camera>) -> T) -> T {
    f(&mut CAMERA.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The main hand is at the camera: its trigger and grip are the camera's, not attack and hotbar.
pub(crate) fn vr_camera_touched() -> bool {
    camera(|c| c.as_ref().is_some_and(|c| c.near || c.grab.is_some()))
}

/// VR stopped or the world left: the camera is put away.
pub(crate) fn vr_camera_reset() {
    camera(|c| *c = None);
    horizon_vr::set_camera(None);
}

/// The main hand's world position and rotation.
fn hand(data: &VrData) -> Option<([f64; 3], Quat)> {
    let aim = data.poses.aim[0]?;
    Some((data.controller_pos(0), data.room.turn() * aim.rotation()))
}

impl Game<'_> {
    /// `toggleHandheldCam`: out in front of the main hand, or put away.
    pub(super) fn vr_toggle_camera(&mut self) {
        let Some(data) = super::vr_player::vr_player().as_ref().and_then(|v| v.pre) else {
            return;
        };
        let placed = camera(|c| {
            if c.take().is_some() {
                return None;
            }
            let (pos, rot) = hand(&data)?;
            let ahead = rot * Vec3::new(0.0, 0.0, -0.2);
            let cam = Camera {
                pos: [pos[0] + f64::from(ahead.x), pos[1] + f64::from(ahead.y), pos[2] + f64::from(ahead.z)],
                rot,
                grab: None,
                near: false,
                buttons: [false; 2],
            };
            let pose = (cam.pos, cam.rot);
            *c = Some(cam);
            Some(pose)
        });
        horizon_vr::set_camera(placed);
    }

    /// `CameraTracker` each tick: grabbing moves the camera with the hand; the trigger at it
    /// takes a photo into the screenshots folder.
    // shortcut: moved at the tick rate (20 Hz), not each frame as Vivecraft's grab does.
    pub(super) fn vr_camera_tick(&mut self, data: VrData) {
        let Some((hpos, hrot)) = hand(&data) else {
            return;
        };
        let pad = horizon_vr::pad();
        let grip = pad.is_some_and(|p| p.right_bumper) && !horizon_vr::gui_open();
        let trigger = pad.is_some_and(|p| p.triggers[1] > 0.5) && !horizon_vr::gui_open();
        let (photo, pose) = camera(|c| {
            let Some(cam) = c.as_mut() else {
                return (false, None);
            };
            let d = [cam.pos[0] - hpos[0], cam.pos[1] - hpos[1], cam.pos[2] - hpos[2]];
            cam.near = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < REACH;
            let [was_grip, was_trigger] = std::mem::replace(&mut cam.buttons, [grip, trigger]);
            if grip && !was_grip && cam.near {
                let local = hrot.inverse() * Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32);
                cam.grab = Some((local, hrot.inverse() * cam.rot));
                horizon_vr::haptic(0, 1000);
            }
            if !grip {
                cam.grab = None;
            }
            if let Some((local, rel)) = cam.grab {
                let o = hrot * local;
                cam.pos = [hpos[0] + f64::from(o.x), hpos[1] + f64::from(o.y), hpos[2] + f64::from(o.z)];
                cam.rot = (hrot * rel).normalize();
            }
            let photo = trigger && !was_trigger && cam.near && cam.grab.is_none();
            (photo, Some((cam.pos, cam.rot)))
        });
        horizon_vr::set_camera(pose);
        if photo {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let path = horizon_client::logging::player_dir()
                .join("screenshots")
                .join(format!("vr_camera_{secs}.png"));
            horizon_vr::take_photo(path);
            horizon_vr::haptic(0, 3000);
        }
    }
}
