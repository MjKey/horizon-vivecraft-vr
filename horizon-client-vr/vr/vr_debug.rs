//! Vivecraft's device debug axes (`DebugRenderHelper.renderDeviceAxes`, the `renderDebugDeviceAxes`
//! and `renderDebugPlayerAxes` debug settings), ported from VivecraftMod (LGPL-3.0): each tracked
//! device's x (red), y (green) and z (blue) axes, 10 cm long, for this player and other VR players.
use super::Game;
use super::vr_player::vr_player;

impl Game<'_> {
    pub(crate) fn vr_debug_geometry(&self, camera: [f64; 3], out: &mut Vec<super::Vertex>) {
        let uv = horizon_client::gfx::lightning::white_uv(&self.gui.atlas.font.white());
        // Vivecraft's headset on other VR players' heads (`VRPlayerModel` HMD layer).
        // shortcut: a plain dark box, not Vivecraft's textured headset model; swap in the model if it looks wrong.
        for m in &self.mobs {
            let Some(p) = self.remote_player_uuid(m.id).and_then(super::vr_net::vr_net_player) else {
                continue;
            };
            let at = glam::Vec3::new(
                (m.position.x + f64::from(p.hmd.position.x) - camera[0]) as f32,
                (m.position.y + f64::from(p.hmd.position.y) - camera[1]) as f32,
                (m.position.z + f64::from(p.hmd.position.z) - camera[2]) as f32,
            );
            let q = p.hmd.orientation;
            let dark = u32::from_le_bytes([30, 30, 34, 255]);
            // Visor in front of the eyes, strap around the head.
            super::vr_teleport::turned_box(out, at + q * glam::Vec3::new(0.0, 0.0, -0.27), q, glam::Vec3::new(0.14, 0.07, 0.04), dark, uv);
            super::vr_teleport::turned_box(out, at + q * glam::Vec3::new(0.0, 0.02, 0.0), q, glam::Vec3::new(0.27, 0.025, 0.27), dark, uv);
        }
        let settings = super::vr_settings::vr_settings();
        let mut devices: Vec<([f64; 3], glam::Quat)> = Vec::new();
        if settings.debug_device_axes
            && let Some(poses) = horizon_vr::poses()
            && let Some(data) = vr_player().as_ref().map(|vr| vr.data(poses))
        {
            let turn = data.room.turn();
            devices.push((data.hmd_pos(), turn * poses.hmd.rotation()));
            for c in 0..2 {
                if let Some(p) = poses.aim[c] {
                    devices.push((data.controller_pos(c), turn * p.rotation()));
                }
            }
        }
        if settings.debug_player_axes {
            for m in &self.mobs {
                let Some(p) = self.remote_player_uuid(m.id).and_then(super::vr_net::vr_net_player) else {
                    continue;
                };
                let feet = [m.position.x, m.position.y, m.position.z];
                for d in [p.hmd, p.main_hand, p.off_hand] {
                    let at = std::array::from_fn(|i| feet[i] + f64::from(d.position[i]));
                    devices.push((at, d.orientation));
                }
            }
        }
        if devices.is_empty() {
            return;
        }
        for (at, q) in devices {
            let o = glam::Vec3::new((at[0] - camera[0]) as f32, (at[1] - camera[1]) as f32, (at[2] - camera[2]) as f32);
            for (axis, rgb) in [(glam::Vec3::X, [255u8, 0, 0]), (glam::Vec3::Y, [0, 255, 0]), (glam::Vec3::Z, [0, 0, 255])] {
                let color = u32::from_le_bytes([rgb[0], rgb[1], rgb[2], 255]);
                // Five small cubes along the axis.
                for k in 1..=5 {
                    let c = o + q * axis * (0.02 * k as f32);
                    super::vr_teleport::cube(out, c.to_array(), 0.004, color, uv);
                }
            }
        }
    }
}
