//! Vivecraft's radial menu, ported from VivecraftMod (LGPL-3.0,
//! https://github.com/Vivecraft/VivecraftMod, `client_vr/gameplay/screenhandlers/RadialHandler.java`,
//! `GuiRadial`), with its defaults: `radialModeHold` (hold B, point, let go), `radialRepeat`,
//! 8 buttons of `getRadialItemsDefault`.
use super::{Game, Screen};
use glam::{Quat, Vec3};
use std::sync::Mutex;
use std::time::Instant;

/// `getRadialItemsDefault`'s first 8, clockwise from the top.
const ITEMS: [&str; 8] = [
    "Drop Selected Item",
    "Chat",
    "Rotate Right",
    "Pick Block",
    "Toggle Handheld Camera",
    "Toggle Player List",
    "Rotate Left",
    "Quick Torch",
];
/// The ring's radius and a button's reach, in the room (meters).
const RADIUS: f32 = 0.22;
const BUTTON: f32 = 0.09;

#[derive(Default)]
struct Radial {
    /// `POS_ROOM`, `ROTATION_ROOM` while showing.
    showing: Option<(Vec3, Quat, Instant)>,
    held: bool,
    trigger: bool,
    last_press: Option<usize>,
    /// Quick torch: the slot to go back to on the next tick.
    restore: Option<usize>,
}

/// `togglePlayerList`: the tab list stays shown until toggled off.
static PLAYER_LIST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn vr_player_list_shown() -> bool {
    PLAYER_LIST.load(std::sync::atomic::Ordering::Relaxed)
}

static RADIAL: Mutex<Option<Radial>> = Mutex::new(None);

/// Closes the transient radial state when VR stops.
pub(crate) fn vr_radial_reset() {
    *RADIAL.lock().unwrap_or_else(|e| e.into_inner()) = None;
    PLAYER_LIST.store(false, std::sync::atomic::Ordering::Relaxed);
}

fn radial<T>(f: impl FnOnce(&mut Radial) -> T) -> T {
    f(RADIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Default::default))
}

/// Whether the radial menu shows (B is the radial key in game then, not the pad's B).
pub(crate) fn vr_radial_showing() -> bool {
    radial(|r| r.showing.is_some())
}

/// Button `i`'s centre in the ring's plane (x right, y up).
fn button(i: usize) -> (f32, f32) {
    let a = i as f32 * std::f32::consts::TAU / ITEMS.len() as f32;
    (a.sin() * RADIUS, a.cos() * RADIUS)
}

/// The button the main controller's laser points at.
fn pointed(pos: Vec3, rot: Quat) -> Option<usize> {
    let aim = horizon_vr::poses()?.aim[0]?;
    let normal = rot * Vec3::Z;
    let dir = aim.direction();
    let denom = dir.dot(normal);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = (pos - aim.translation()).dot(normal) / denom;
    if t < 0.0 {
        return None;
    }
    let local = rot.inverse() * (aim.translation() + dir * t - pos);
    (0..ITEMS.len())
        .map(|i| {
            let (x, y) = button(i);
            (i, (local.x - x).hypot(local.y - y))
        })
        .filter(|&(_, d)| d < BUTTON)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

impl Game<'_> {
    /// `RadialHandler` each tick: B opens the ring in front of the main controller; letting go
    /// presses the button the laser is on (or, `radialRepeat`, the last one when let go at once).
    pub(super) fn vr_radial_tick(&mut self) {
        if let Some(slot) = radial(|r| r.restore.take()) {
            self.hotbar_presses.push(slot);
        }
        let down = horizon_vr::pad().is_some_and(|p| p.east)
            && self.screen == Screen::Playing
            && !horizon_vr::gui_open();
        let was = radial(|r| std::mem::replace(&mut r.held, down));
        let settings = super::vr_settings::vr_settings();
        let showing = vr_radial_showing();
        // Press mode (`radialModeHold` off): B opens and closes it, the trigger presses.
        if !settings.radial_hold {
            let trigger = horizon_vr::pad().is_some_and(|p| p.triggers[1] > 0.5);
            let was_trigger = radial(|r| std::mem::replace(&mut r.trigger, trigger));
            if down && !was {
                if showing {
                    radial(|r| r.showing = None);
                } else {
                    self.vr_radial_open(false);
                }
            } else if showing && trigger && !was_trigger {
                if let Some(i) = radial(|r| r.showing).and_then(|(pos, rot, _)| pointed(pos, rot)) {
                    radial(|r| r.last_press = Some(i));
                    self.vr_radial_press(i);
                }
            }
            return;
        }
        if down && !was {
            self.vr_radial_open(true);
        } else if !down && was {
            let Some((pos, rot, at)) = radial(|r| r.showing.take()) else {
                return;
            };
            let press = pointed(pos, rot).or_else(|| {
                radial(|r| r.last_press).filter(|_| settings.radial_repeat && at.elapsed().as_millis() < 500)
            });
            if let Some(i) = press {
                radial(|r| r.last_press = Some(i));
                self.vr_radial_press(i);
            }
        }
    }

    /// `orientOverlay`: hold mode 0.6 m along the main controller, else 1 m ahead of the head;
    /// facing back.
    fn vr_radial_open(&mut self, hold: bool) {
        let Some(poses) = horizon_vr::poses() else {
            return;
        };
        let Some(from) = (if hold { poses.aim[0] } else { Some(poses.hmd) }) else {
            return;
        };
        let offset = from.direction() * if hold { 0.6 } else { 1.0 };
        let pitch = (offset.y / offset.length()).asin();
        let yaw = std::f32::consts::PI + offset.x.atan2(offset.z);
        let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
        // `LAST_PRESS` stays: `radialRepeat` presses it again on a quick release.
        radial(|r| r.showing = Some((from.translation() + offset, rot, Instant::now())));
    }

    fn vr_radial_press(&mut self, i: usize) {
        let increment = super::vr_player::settings::world_rotation_increment().max(10.0);
        match i {
            0 => drop(self.click_mapping("key.drop")),
            1 => self.open_chat_method(""),
            2 | 6 => {
                if let Some(vr) = super::vr_player::vr_player().as_mut() {
                    let turn = if i == 2 { -increment } else { increment };
                    vr.world_rotation = (vr.world_rotation + turn) % 360.0;
                }
            }
            3 => drop(self.click_mapping("key.pickItem")),
            7 => {
                // `VivecraftVRMod.keyQuickTorch`: the first torch in the hotbar, used, and back.
                let torch =
                    (0..9).find(|&s| self.stack_in(s).is_some_and(|t| t.item.ends_with("torch")));
                if let Some(slot) = torch {
                    let back = self.selected;
                    self.hotbar_presses.push(slot);
                    drop(self.click_mapping("key.use"));
                    radial(|r| r.restore = Some(back));
                }
            }
            5 => {
                PLAYER_LIST.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
            }
            4 => self.vr_toggle_camera(),
            _ => log::info!("Horizon VR: radial \"{}\" is not ported", ITEMS[i]),
        }
    }

    /// The ring in the world: a label per button facing the head, the pointed one bright.
    pub(crate) fn vr_radial_geometry(&mut self, camera: [f64; 3], out: &mut Vec<super::Vertex>) {
        let Some((pos, rot, _)) = radial(|r| r.showing) else {
            return;
        };
        let Some(room) = horizon_vr::render_room() else {
            return;
        };
        let lit = pointed(pos, rot);
        let head = horizon_vr::poses().map(|p| room.to_world(p.hmd.translation()));
        let posed = self.posed_rotation;
        let mut patches = Vec::new();
        for (i, name) in ITEMS.iter().enumerate() {
            let (x, y) = button(i);
            let w = room.to_world(pos + rot * Vec3::new(x, y, 0.0));
            let o = glam::DVec3::new(w[0] - camera[0], w[1] - camera[1], w[2] - camera[2]);
            // Each label faces the head (`GuiRadial` is a panel facing it).
            if let Some(h) = head {
                // `posed_rotation` is a camera's look (`name_tag_quads`' forward): head to label.
                let d = Vec3::new(
                    (w[0] - h[0]) as f32,
                    (w[1] - h[1]) as f32,
                    (w[2] - h[2]) as f32,
                );
                self.posed_rotation =
                    Some((super::vr_player::yaw_of(d), -super::vr_player::pitch_of(d)));
            }
            let alpha = if lit == Some(i) { 0xFF } else { 0x9F };
            let text = horizon_client::text::Text::literal(*name);
            patches.extend(self.name_tag_quads(
                o,
                &[(text, 0.0)],
                alpha,
                true,
                0,
                horizon_client::block_entity_render::vertex_light(0xFF),
                0.0035,
                out,
            ));
        }
        self.posed_rotation = posed;
        self.map_patches.extend(patches);
    }
}
