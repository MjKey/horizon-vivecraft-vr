//! Vivecraft's hands in view, ported from VivecraftMod (LGPL-3.0,
//! https://github.com/Vivecraft/VivecraftMod): `RenderHelper.setupRenderingAtController`,
//! `VRArmHelper.renderVRHand_Main/Offhand`, the first person arm
//! (`FirstPersonHandsAndItemsRendererVRMixin.vrPlayerArm`) and the held item's transforms
//! (`VivecraftItemRendering.applyFirstPersonItemTransforms`).
use super::Game;
use glam::{Mat4, Quat, Vec3};

/// One hand's pose for drawing: the controller (`getController(c)`, its aim) relative to `eye`,
/// scaled by the world scale, and the controller's gun angle (`MCVR.getGunAngle`).
#[derive(Clone, Copy)]
pub(super) struct VrHand {
    pub to_world: Mat4,
    pub gun_angle: f32,
}

/// `VivecraftItemTransformType`, by what the hand holds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum ItemTransform {
    Item,
    Block3d,
    BlockStick,
    Noms,
    Map,
    Bow,
    BowDrawing,
    Horn,
    Mace,
    Sword,
    Shield,
    Spear,
    Crossbow,
    Compass,
    Tool,
    ToolRod,
    Arrow,
}

fn rx(d: f32) -> Quat {
    Quat::from_rotation_x(d.to_radians())
}
fn ry(d: f32) -> Quat {
    Quat::from_rotation_y(d.to_radians())
}
fn rz(d: f32) -> Quat {
    Quat::from_rotation_z(d.to_radians())
}

/// `applyFirstPersonItemTransforms`: the item's place in the hand.
pub(super) fn item_transform(kind: ItemTransform, main_hand: bool, gun: f32, use_ticks: f32) -> Mat4 {
    let mut scale = 0.7;
    let (mut tx, mut ty, mut tz) = (-0.05f32, 0.005f32, 0.0f32);
    let mut pre = Quat::IDENTITY;
    let mut rotation = rx(-110.0 + gun);
    match kind {
        ItemTransform::Bow => {
            pre = rotation;
            rotation = Quat::IDENTITY;
            tx -= 0.0225;
            ty -= 0.25;
            tz += 0.025 + 0.03 * gun / 40.0;
            scale = 1.0;
        }
        ItemTransform::BowDrawing => {
            pre = rotation;
            rotation = Quat::IDENTITY;
            tx -= 0.0225;
            ty -= 0.25;
            tz += 0.025 + 0.03 * gun / 40.0;
            scale = 1.0;
        }
        ItemTransform::Crossbow => {
            rotation = ry(10.0);
            tx += 0.01;
            tz -= 0.02;
            ty -= 0.02;
            scale = 0.5;
        }
        ItemTransform::Map => {
            rotation = rx(-45.0);
            tx = 0.0;
            ty = 0.16;
            tz = -0.075;
            scale = 0.75;
        }
        ItemTransform::Noms => {
            rotation = rz(180.0) * rx(-135.0);
            tx += 0.08;
            tz += 0.02 + 0.006 * use_ticks.sin();
            scale = 0.4;
        }
        ItemTransform::Item => {
            rotation = rz(180.0) * rx(-135.0);
            tx += 0.08;
            tz += -0.08;
            scale = 0.4;
        }
        ItemTransform::Compass => {
            rotation = ry(90.0) * rx(25.0);
            scale = 0.4;
        }
        ItemTransform::Block3d => {
            tx += 0.05;
            tz -= 0.1;
            rotation *= rx(90.0 - gun);
            scale = 0.3;
        }
        ItemTransform::BlockStick | ItemTransform::Horn => {
            rotation = rx(-45.0 + gun);
            ty += -0.105 + 0.06 * gun / 40.0;
            tz -= 0.1;
            if kind == ItemTransform::Horn {
                scale = 0.3;
            }
        }
        ItemTransform::Shield => {
            let side = if main_hand { 1.0 } else { -1.0 };
            scale = 0.4;
            ty += 0.18;
            if side > 0.0 {
                rotation *= rx(105.0 - gun);
                tx += 0.11;
            } else {
                rotation *= rx(115.0 - gun);
                tx -= 0.015;
            }
            tz += 0.1;
            rotation *= ry(side * -90.0);
        }
        ItemTransform::Spear => {
            rotation = Quat::IDENTITY;
            tx -= 0.135;
            tz += 0.575;
            scale = 0.6;
            ty += 0.2 * gun / 40.0;
            rotation *= rx(gun);
            rotation *= rx(-65.0);
            tz += -0.75;
        }
        ItemTransform::ToolRod => {
            rotation *= rx(40.0);
            ty += -0.02 + gun / 40.0 * 0.1;
            tx += 0.05;
            tz -= 0.15;
            scale = 0.8;
        }
        ItemTransform::Arrow => {
            pre = rz(-180.0);
            rotation *= rx(-gun);
        }
        ItemTransform::Mace => {
            pre = rx(gun);
            rotation *= rx(-gun);
            tx = 0.0;
            ty = 0.0125;
            tz = -0.06;
            scale = 0.56;
        }
        ItemTransform::Sword | ItemTransform::Tool => {}
    }
    Mat4::from_quat(pre)
        * Mat4::from_translation(Vec3::new(tx, ty, tz))
        * Mat4::from_quat(rotation)
        * Mat4::from_scale(Vec3::splat(scale))
}

/// The first person arm at the controller (`vivecraft$vrPlayerArm`): along the controller,
/// shoulder end away from the hand.
pub(super) fn arm_transform(right_hand: bool, slim: bool) -> Mat4 {
    let dir = if right_hand { 1.0 } else { -1.0 };
    Mat4::from_scale(Vec3::splat(0.4))
        * Mat4::from_translation(Vec3::new(if slim { -0.34375 } else { -0.375 } * dir, 0.0, 0.75))
        * Mat4::from_quat(rx(-90.0))
        * Mat4::from_quat(ry(180.0))
}

impl Game<'_> {
    /// Where to draw each hand (main, offhand) while VR shows the game, relative to the eyes the
    /// hand geometry is built around.
    pub(super) fn vr_hands(&self) -> Option<[Option<VrHand>; 2]> {
        if !self.vr_running() {
            return None;
        }
        let poses = horizon_vr::poses()?;
        let room = horizon_vr::render_room()?;
        let eye = self.eye_position()?;
        Some([0, 1].map(|c| {
            let aim = poses.aim[c]?;
            let at = room.to_world(aim.translation());
            let offset = Vec3::new(
                (at[0] - eye[0]) as f32,
                (at[1] - eye[1]) as f32,
                (at[2] - eye[2]) as f32,
            );
            let rotation = room.turn() * aim.rotation();
            // The angle between where the controller points and how the hand holds it.
            let gun_angle = poses.grip[c].map_or(0.0, |g| {
                aim.direction().dot(g.direction()).abs().min(1.0).acos().to_degrees()
            });
            Some(VrHand {
                to_world: Mat4::from_translation(offset)
                    * Mat4::from_quat(rotation)
                    * Mat4::from_scale(Vec3::splat(room.scale)),
                gun_angle,
            })
        }))
    }

    /// `VivecraftItemRendering.getTransformType`.
    pub(super) fn vr_item_transform_kind(&self, item: &str, main_hand: bool) -> ItemTransform {
        use horizon_core::use_animation::UseAnimation;
        let n = item.trim_start_matches("minecraft:");
        let animation = self
            .vr_stack_for(main_hand)
            .map_or(UseAnimation::None, |s| self.item_use_animation(&s));
        match animation {
            UseAnimation::Eat | UseAnimation::Drink => return ItemTransform::Noms,
            UseAnimation::Bow => {
                let drawing = super::vr_player::vr_player()
                    .as_ref()
                    .is_some_and(|v| v.trackers.bow.drawing);
                return if drawing { ItemTransform::BowDrawing } else { ItemTransform::Bow };
            }
            UseAnimation::TootHorn => return ItemTransform::Horn,
            _ => {}
        }
        if n.ends_with("torch") || n == "lantern" || n == "soul_lantern" {
            return ItemTransform::BlockStick;
        }
        if n.ends_with("filled_map") || n == "map" {
            return ItemTransform::Map;
        }
        if n == "mace" {
            return ItemTransform::Mace;
        }
        if n.ends_with("_sword") {
            return ItemTransform::Sword;
        }
        if n == "shield" {
            return ItemTransform::Shield;
        }
        if n == "trident" {
            return ItemTransform::Spear;
        }
        if n == "crossbow" {
            return ItemTransform::Crossbow;
        }
        if n.ends_with("compass") || n == "clock" {
            return ItemTransform::Compass;
        }
        if n == "fishing_rod" || n.ends_with("_on_a_stick") {
            return ItemTransform::ToolRod;
        }
        if n.ends_with("arrow") {
            return ItemTransform::Arrow;
        }
        if n.ends_with("_pickaxe")
            || n.ends_with("_axe")
            || n.ends_with("_shovel")
            || n.ends_with("_hoe")
            || matches!(n, "shears" | "bone" | "blaze_rod" | "bamboo" | "stick" | "debug_stick" | "flint_and_steel" | "brush")
        {
            return ItemTransform::Tool;
        }
        // A block's item (`BlockItem`) is held as a block.
        if self.pack.assets.registry.default_state(item).is_some() {
            return ItemTransform::Block3d;
        }
        ItemTransform::Item
    }

    fn vr_stack_for(&self, main_hand: bool) -> Option<horizon_client::items::ItemStack> {
        if main_hand {
            self.held().cloned()
        } else {
            self.offhand.clone()
        }
    }
}
