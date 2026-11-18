//! Vivecraft's `TeleportTracker`, ported from VivecraftMod (LGPL-3.0,
//! https://github.com/Vivecraft/VivecraftMod, `client_vr/gameplay/trackers/TeleportTracker.java`),
//! with `VRMovementStyle`'s defaults (arc aiming, teleport on release) and limited survival teleport
//! (`vrTeleportUpLimit` 1, `vrTeleportDownLimit` 4, `vrTeleportHorizLimit` 16).
//! The teleport key is the right stick pushed forward; the arc comes from the offhand (left) controller.
use super::vr_player::{VrData, vr_player};
use super::{Game, Hit, Screen};
use horizon_core::geometry::{Aabb, Vec3 as WVec3};

const MAX_STEPS: usize = 50;
const UP_LIMIT: f64 = 1.0;
const DOWN_LIMIT: f64 = 4.0;
const HORIZ_LIMIT: f64 = 16.0;
/// A vanilla server takes a move this long without "moved too quickly"; longer goes as `/tp`.
const DIRECT_MOVE: f64 = 9.5;

#[derive(Default)]
pub(super) struct TeleportTracker {
    energy: f32,
    destination: Option<[f64; 3]>,
    progress: f64,
    timer: i32,
    distance: f64,
    /// The arc in the world, for drawing.
    pub arc: Vec<[f64; 3]>,
}

impl TeleportTracker {
    pub fn is_aiming(&self) -> bool {
        self.progress > 0.0
    }
}

/// Whether the teleport key is held: the right stick pushed forward, no menu open.
fn teleport_key() -> bool {
    super::vr_settings::vr_settings().teleport
        && !super::vr_net::server_forbids_teleport()
        && !super::vr_radial::vr_radial_showing()
        && !horizon_vr::gui_open() && horizon_vr::pad().is_some_and(|p| p.right[1] > 0.7)
}

impl Game<'_> {
    /// `TeleportTracker.isActive` + `activeProcess`, then `updateTeleportDestinations`.
    pub(super) fn vr_teleport_tracker(&mut self, data: VrData) -> horizon_client::Result<()> {
        let active = self.screen != Screen::Dead && self.player.is_some();
        if !active {
            if let Some(vr) = vr_player().as_mut() {
                let t = &mut vr.trackers.teleport;
                t.destination = None;
                t.arc.clear();
                t.progress = 0.0;
            }
            return Ok(());
        }
        let key = teleport_key() && self.riding.is_none();
        let Some(pos) = self.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]) else {
            return Ok(());
        };
        let mut do_teleport = None;
        {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            let t = &mut vr.trackers.teleport;
            if t.energy < 100.0 {
                t.energy += 1.0;
            }
            if key {
                // teleportOnRelease: the progress is full while the key is held.
                t.timer += 1;
                if t.timer > 0 {
                    t.progress = 1.0;
                }
            } else {
                if t.progress >= 1.0 {
                    do_teleport = t.destination;
                }
                t.timer = 0;
                t.progress = 0.0;
            }
        }
        if let Some(dest) = do_teleport {
            let d = [dest[0] - pos[0], dest[1] - pos[1], dest[2] - pos[2]];
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if super::vr_net::server_direct_teleport() {
                // `TeleportPayloadC2S`: the Vivecraft server moves the player, any distance.
                self.vr_set_player_pos(dest);
                self.vr_net_teleport(dest);
            } else if distance <= DIRECT_MOVE {
                self.vr_set_player_pos(dest);
            } else {
                // `isTeleportSupported` false on a vanilla server: the teleport is a command.
                self.send_command(&format!("tp {} {} {}", dest[0], dest[1], dest[2]));
            }
            // doTeleportCallback
            if let Some(p) = self.player.as_mut() {
                p.fall_distance = 0.0;
                p.velocity = Default::default();
            }
            let creative = self.creative;
            if let Some(vr) = vr_player().as_mut() {
                vr.trackers.swing.disable_swing = 3;
                let t = &mut vr.trackers.teleport;
                if !creative {
                    t.energy -= distance as f32 * 4.0;
                }
                t.distance = distance;
                t.timer = -1;
            }
        }
        self.vr_update_teleport_arc(data)
    }

    /// `updateTeleportArc`: the arc from the offhand controller, falling, to the first block it
    /// hits; a standing place there is the destination.
    fn vr_update_teleport_arc(&mut self, data: VrData) -> horizon_client::Result<()> {
        let (aiming, energy) = match vr_player().as_mut() {
            Some(vr) => {
                let t = &mut vr.trackers.teleport;
                t.destination = None;
                if !t.is_aiming() {
                    t.arc.clear();
                }
                (t.is_aiming(), t.energy)
            }
            None => return Ok(()),
        };
        if !aiming || !data.controller_tracked(1) {
            return Ok(());
        }
        let start = data.controller_pos(1);
        // The controller's down, with its roll: the arc bends the way the hand is held.
        let down = data.controller_vector(1, glam::Vec3::NEG_Y);
        let gravity = [down.x as f64 * 0.098, down.y as f64 * 0.098, down.z as f64 * 0.098];
        let dir = data.controller_dir(1);
        let mut velocity = [dir.x as f64 * 0.5, dir.y as f64 * 0.5, dir.z as f64 * 0.5];
        let mut arc = vec![start];
        let mut p = start;
        let mut destination = None;
        let mut i = 1;
        while i < MAX_STEPS && (i * 4) as f32 <= energy {
            let next = [p[0] + velocity[0], p[1] + velocity[1], p[2] + velocity[2]];
            let len = (velocity[0].powi(2) + velocity[1].powi(2) + velocity[2].powi(2)).sqrt();
            let hit = (len > 1e-9)
                .then(|| self.clip_ray_fluids(p, velocity.map(|v| v / len), len, true))
                .flatten();
            if let Some(hit) = hit {
                arc.push(hit.point);
                destination = self.vr_teleport_destination(&hit)?;
                if let (Some(dest), Some(player)) = (destination, self.player.as_ref()) {
                    let diff = [
                        player.position.x - dest[0],
                        player.position.y - dest[1],
                        player.position.z - dest[2],
                    ];
                    let xz = (diff[0] * diff[0] + diff[2] * diff[2]).sqrt();
                    let mut ok = !player.shift_key_down || diff[1] <= 0.2;
                    let mayfly = self.creative || player.spectator;
                    if !mayfly && super::vr_settings::vr_settings().limited_teleport {
                        // shortcut: jump boost / speed factors (`getMuhJumpFactor`) are taken as 1.
                        if diff[1] > DOWN_LIMIT + 0.2 || -diff[1] > UP_LIMIT + 0.2 || xz > HORIZ_LIMIT + 0.2 {
                            ok = false;
                        }
                    }
                    if !ok {
                        destination = None;
                    }
                }
                break;
            }
            p = next;
            arc.push(next);
            for k in 0..3 {
                velocity[k] += gravity[k];
            }
            i += 1;
        }
        if let Some(vr) = vr_player().as_mut() {
            let t = &mut vr.trackers.teleport;
            t.arc = arc;
            t.destination = destination;
        }
        Ok(())
    }

    /// `checkAndSetTeleportDestination`: a place to stand on what the arc hit.
    fn vr_teleport_destination(&mut self, hit: &Hit) -> horizon_client::Result<Option<[f64; 3]>> {
        let Some(player) = self.player.as_ref() else {
            return Ok(None);
        };
        let half = f64::from(player.dimensions.width) / 2.0;
        let height = f64::from(player.dimensions.height);
        let mayfly = self.creative || player.spectator;
        let at = |x: f64, y: f64, z: f64| {
            Aabb::new(WVec3::new(x - half, y, z - half), WVec3::new(x + half, y + height, z + half))
        };
        let name = self.vr_block_name(hit.pos);
        let [bx, by, bz] = hit.pos.map(f64::from);
        if name.ends_with("water") || name.ends_with("lava") {
            for (x, z) in [(hit.point[0], hit.point[2]), (bx + 0.5, bz + 0.5)] {
                if self.vr_no_collision(at(x, by, z))? {
                    return Ok(Some([x, by, z]));
                }
            }
            return Ok(None);
        }
        let up = hit.face == [0, 1, 0];
        if !up {
            if name.ends_with("ladder") || name.ends_with(":vine") {
                let below = self.vr_block_name([hit.pos[0], hit.pos[1] - 1, hit.pos[2]]);
                let y = if below == name { by - 0.5 } else { by + 0.5 };
                return Ok(Some([bx + 0.5, y, bz + 0.5]));
            }
            if !mayfly {
                return Ok(None);
            }
        }
        // shortcut: the top is the hit point on a top face, else the block's full top; partial
        // shapes hit from the side land on y + 1.
        let top = if up { hit.point[1] } else { by + 1.0 };
        let extra = if name.ends_with("soul_sand") || name.ends_with("honey_block") { 0.05 } else { 0.0 };
        for (x, z) in [(hit.point[0], hit.point[2]), (bx + 0.5, bz + 0.5)] {
            let bb = at(x, top, z);
            let grow = 0.125 + extra;
            let below = Aabb::new(
                WVec3::new(x - half, top - grow, z - half),
                WVec3::new(x + half, top + height + grow, z + half),
            );
            if self.vr_no_collision(bb)? && !self.vr_no_collision(below)? {
                return Ok(Some([x, top, z]));
            }
        }
        Ok(None)
    }

    /// The arc, as small boxes in the frame's entity vertices (Vivecraft draws it as a tube).
    pub(crate) fn vr_teleport_arc_geometry(&self, camera: [f64; 3], out: &mut Vec<super::Vertex>) {
        let (arc, ok) = match vr_player().as_ref() {
            Some(vr) if vr.trackers.teleport.is_aiming() => {
                let t = &vr.trackers.teleport;
                (t.arc.clone(), t.destination.is_some())
            }
            _ => return,
        };
        let uv = horizon_client::gfx::lightning::white_uv(&self.gui.atlas.font.white());
        // Vivecraft's arc: blue when it can land, red when not.
        let [r, g, b] = if ok { [80u8, 160, 255] } else { [255u8, 60, 60] };
        let color = u32::from_le_bytes([r, g, b, 200]);
        for p in &arc {
            let c = [0, 1, 2].map(|i| (p[i] - camera[i]) as f32);
            cube(out, c, 0.03, color, uv);
        }
    }
}

/// A cube of half-size `s` at `c` (camera-relative), lit full, in the translucent overlay layer.
pub(crate) fn cube(out: &mut Vec<super::Vertex>, c: [f32; 3], s: f32, color: u32, uv: [f32; 2]) {
    turned_box(out, c.into(), glam::Quat::IDENTITY, glam::Vec3::splat(s), color, uv);
}

/// A box at `c`, turned by `q`, `half` its half-size on each axis.
pub(crate) fn turned_box(out: &mut Vec<super::Vertex>, c: glam::Vec3, q: glam::Quat, half: glam::Vec3, color: u32, uv: [f32; 2]) {
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for high in [false, true] {
            for (hu, hv) in [(false, false), (true, false), (true, true), (false, true)] {
                let mut d = glam::Vec3::ZERO;
                d[axis] = if high { half[axis] } else { -half[axis] };
                d[u] = if hu { half[u] } else { -half[u] };
                d[v] = if hv { half[v] } else { -half[v] };
                out.push(super::Vertex {
                    pos: (c + q * d).to_array(),
                    uv,
                    layer: horizon_client::gfx::TRANSLUCENT_PARTICLE_OVERLAY,
                    color,
                    light: 240 | 240 << 8,
                    normal: [0; 4],
                    glint_uv: [0.0; 2],
                });
            }
        }
    }
}
