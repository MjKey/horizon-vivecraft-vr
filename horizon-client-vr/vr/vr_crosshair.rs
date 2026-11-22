//! Vivecraft's in-world crosshair (`renderInGameCrosshairMode` ALWAYS, ported from VivecraftMod,
//! LGPL-3.0, `VREffectsHelper.renderCrosshairAtDepth`): in VR there is no screen centre, so the `hud/crosshair` sprite
//! lies on the block face the main controller's laser hits, flat against it.
use super::Game;

impl Game<'_> {
    pub(crate) fn vr_crosshair_geometry(&mut self, camera: [f64; 3], out: &mut Vec<super::Vertex>) {
        if self.screen != super::Screen::Playing
            || !super::vr_settings::vr_settings().crosshair
            || horizon_vr::poses().is_none()
            || super::vr_radial::vr_radial_showing()
            || self.ui.hide_gui
        {
            return;
        }
        let Some(hit) = self.pick() else {
            return;
        };
        let n = hit.face.map(f64::from);
        // Just off the face, so it isn't hidden in it.
        let at = std::array::from_fn::<f64, 3, _>(|i| hit.point[i] + n[i] * 0.01 - camera[i]);
        let Some(r) = self.gui.atlas.sprites.get("hud/crosshair").copied() else {
            return;
        };
        let o = glam::Vec3::from_array(at.map(|v| v as f32));
        // Two axes in the face's plane; Vivecraft's crosshair is 0.1 blocks across at 1 m.
        let n = glam::Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32);
        let side = if n.y.abs() > 0.5 { glam::Vec3::X } else { glam::Vec3::Y };
        let half = 0.05 * o.length().max(0.5);
        let a = n.cross(side).normalize() * half;
        let b = n.cross(a).normalize() * half;
        let light = horizon_client::block_entity_render::vertex_light(0xFF);
        // Same order as `name_tag_quads`: the corners from the bottom left.
        for (p, uv) in [
            (o - a - b, [r.u0, r.v1]),
            (o + a - b, [r.u1, r.v1]),
            (o + a + b, [r.u1, r.v0]),
            (o - a + b, [r.u0, r.v0]),
        ] {
            out.push(super::Vertex {
                normal: [0; 4],
                pos: p.to_array(),
                uv,
                layer: 0,
                color: u32::MAX,
                light,
                glint_uv: [0.0, 0.0],
            });
        }
    }
}
