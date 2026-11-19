//! Vivecraft's `VRPlayer` and `VRData`, ported from VivecraftMod (LGPL-3.0,
//! https://github.com/Vivecraft/VivecraftMod, `client_vr/gameplay/VRPlayer.java`, `client_vr/VRData.java`).
//!
//! The room (`roomOrigin`, `worldRotation`, `worldScale`) sits in the world: every tracked device's
//! world pose is the room's transform of its pose in the room (`horizon_vr::Room`). The player
//! follows the headset around the room (`doPlayerMoveInRoom`), the room follows the player when the
//! game moves them (`LocalPlayerVRMixin.move`/`setPos`), the right stick turns the room about the
//! head (`rotateOriginAround`), and the player looks where the main controller aims
//! (`doPermanentLookOverride`) while walking goes where the offhand controller points
//! (`freemoveDirection`, standing `FreeMove.CONTROLLER`).
use super::{Game, Screen};
use glam::Vec3;
use horizon_core::geometry::{Aabb, Axis, Vec3 as WVec3};
use horizon_vr::{Poses, Room};
use std::sync::{Mutex, MutexGuard};

/// `VRSettings` defaults the port follows.
pub(super) mod settings {
    use super::super::vr_settings::vr_settings;
    /// `worldRotationIncrement`: degrees per snap turn, 0 smooth.
    pub fn world_rotation_increment() -> f32 {
        vr_settings().rotation_increment
    }
    /// `walkUpBlocks`: roomscale steps up blocks.
    pub fn walk_up_blocks() -> bool {
        vr_settings().walk_up_blocks
    }
}

/// One tick's tracking in the world: `VRData` (the room's transform of the tracked poses).
#[derive(Clone, Copy, Debug)]
pub(super) struct VrData {
    pub poses: Poses,
    pub room: Room,
}

impl VrData {
    fn pos(&self, p: Vec3) -> [f64; 3] {
        self.room.to_world(p)
    }
    pub fn hmd_pos(&self) -> [f64; 3] {
        self.pos(self.poses.hmd.translation())
    }
    pub fn hmd_dir(&self) -> Vec3 {
        self.room.dir_to_world(self.poses.hmd.direction())
    }
    /// `getController(c)`: the controller's aim, else the headset's.
    pub fn controller_pos(&self, c: usize) -> [f64; 3] {
        self.poses.aim[c].map_or(self.hmd_pos(), |p| self.pos(p.translation()))
    }
    pub fn controller_dir(&self, c: usize) -> Vec3 {
        self.poses.aim[c].map_or(self.hmd_dir(), |p| self.room.dir_to_world(p.direction()))
    }
    pub fn controller_tracked(&self, c: usize) -> bool {
        self.poses.aim[c].is_some()
    }
    /// A controller-local vector in the world (`VRDevicePose.getCustomVector`).
    pub fn controller_vector(&self, c: usize, axis: Vec3) -> Vec3 {
        let rotation = self.poses.aim[c].map_or(self.poses.hmd.rotation(), |p| p.rotation());
        self.room.turn() * rotation * axis
    }
    /// `getHand(c)`: the grip, else the aim.
    pub fn hand_pos(&self, c: usize) -> [f64; 3] {
        self.poses.grip[c].map_or(self.controller_pos(c), |p| self.pos(p.translation()))
    }
    /// The head's pivot (`getHeadPivotOffset`): 10 cm below and behind the eyes, scaled.
    fn head_pivot_offset(&self, scale: f32) -> Vec3 {
        self.room.turn() * self.poses.hmd.rotation() * Vec3::new(0.0, -0.1 * scale, 0.1 * scale)
    }
    pub fn head_pivot(&self) -> [f64; 3] {
        let o = self.head_pivot_offset(self.room.scale);
        add(self.hmd_pos(), o)
    }
    /// `getNewHeadPivot`: the pivot with another room origin and scale.
    pub fn new_head_pivot(&self, origin: [f64; 3], scale: f32) -> [f64; 3] {
        let eye = self.hmd_pos();
        let moved = std::array::from_fn(|i| eye[i] - self.room.origin[i] + origin[i]);
        add(moved, self.head_pivot_offset(scale))
    }
    /// `getHeadRear`: 20 cm below and behind the eyes.
    pub fn head_rear(&self) -> [f64; 3] {
        let o = self.room.turn()
            * self.poses.hmd.rotation()
            * Vec3::new(0.0, -0.2 * self.room.scale, 0.2 * self.room.scale);
        add(self.hmd_pos(), o)
    }
    /// The headset's height above the room floor, in the room (meters).
    pub fn hmd_height(&self) -> f32 {
        self.poses.hmd.position[1]
    }
    /// `getBodyYaw` (degrees), `MathUtils.estimateBodyYawRad` without full-body trackers: the
    /// headset's yaw, turned toward the hands while both are tracked in front.
    pub fn body_yaw(&self) -> f32 {
        let head = yaw_of(self.hmd_dir());
        if !(self.controller_tracked(0) && self.controller_tracked(1)) {
            return head;
        }
        let h = self.hmd_pos();
        let c0 = self.controller_pos(0);
        let c1 = self.controller_pos(1);
        // The arms' line, turned a quarter right: the way the chest faces between the hands.
        let arms = Vec3::new((c1[0] - c0[0]) as f32, 0.0, (c1[2] - c0[2]) as f32);
        if arms.length_squared() < 1e-6 {
            return head;
        }
        let a = arms.normalize();
        let facing = Vec3::new(-a.z, 0.0, a.x);
        let mid = Vec3::new(
            ((c0[0] + c1[0]) / 2.0 - h[0]) as f32,
            0.0,
            ((c0[2] + c1[2]) / 2.0 - h[2]) as f32,
        );
        // The hands in front of the head: average the chest's and the head's yaw.
        let facing = if facing.dot(mid) < 0.0 {
            -facing
        } else {
            facing
        };
        let chest = yaw_of(facing);
        let diff = wrap_degrees(chest - head);
        if diff.abs() > 90.0 {
            head
        } else {
            head + diff * 0.5
        }
    }
}

fn add(p: [f64; 3], v: Vec3) -> [f64; 3] {
    [
        p[0] + f64::from(v.x),
        p[1] + f64::from(v.y),
        p[2] + f64::from(v.z),
    ]
}

/// A world direction's yaw (`atan2(-x, z)`, degrees), Minecraft's.
pub(super) fn yaw_of(d: Vec3) -> f32 {
    (-d.x).atan2(d.z).to_degrees()
}

/// A world direction's pitch as Vivecraft's `getPitch` (up positive, degrees).
pub(super) fn pitch_of(d: Vec3) -> f32 {
    let len = d.length();
    if len == 0.0 {
        0.0
    } else {
        (d.y / len).clamp(-1.0, 1.0).asin().to_degrees()
    }
}

pub(super) fn wrap_degrees(a: f32) -> f32 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// `VRPlayer`'s state between ticks.
pub(super) struct VrPlayer {
    /// `roomOrigin`.
    pub room_origin: [f64; 3],
    /// `vrSettings.worldRotation`, degrees.
    pub world_rotation: f32,
    /// `worldScale`.
    pub world_scale: f32,
    /// `VehicleTracker`: the player's yaw when the tick began while riding.
    vehicle_yaw: Option<f32>,
    /// The room was placed under the player (`snapRoomOriginToPlayerEntity` at the first tick).
    pub placed: bool,
    /// The local player's join the room was placed for: a new world (or respawn) places it again,
    /// or the room stays where the last world left it (inside blocks: a black view).
    joined: horizon_client::join_seq::JoinSeq,
    /// The right stick was pushed past the snap threshold (`keyRotateAxis.consumeClick`).
    snapped: bool,
    /// `roomScaleMovementDelay`.
    pub room_scale_movement_delay: i32,
    /// `vrdata_world_pre`: this tick's tracking with last tick's room.
    pub pre: Option<VrData>,
    /// `crossVec`: where the main controller's aim reaches.
    pub cross_vec: Option<[f64; 3]>,
    /// The player's position before the tick's physics, for the room to follow.
    before: Option<[f64; 3]>,
    /// Trackers' state.
    pub trackers: super::vr_trackers::Trackers,
}

static VR_PLAYER: Mutex<Option<VrPlayer>> = Mutex::new(None);

/// The port's state while VR runs: `None` until the headset first shows the game.
pub(super) fn vr_player() -> MutexGuard<'static, Option<VrPlayer>> {
    VR_PLAYER.lock().unwrap_or_else(|e| e.into_inner())
}

impl VrPlayer {
    fn new() -> Self {
        Self {
            room_origin: [0.0; 3],
            world_rotation: 0.0,
            vehicle_yaw: None,
            world_scale: 1.0,
            placed: false,
            joined: Default::default(),
            snapped: false,
            room_scale_movement_delay: 0,
            pre: None,
            cross_vec: None,
            before: None,
            trackers: Default::default(),
        }
    }
    pub fn room(&self) -> Room {
        Room {
            origin: self.room_origin,
            rotation: self.world_rotation.to_radians(),
            scale: self.world_scale,
        }
    }
    pub fn data(&self, poses: Poses) -> VrData {
        VrData {
            poses,
            room: self.room(),
        }
    }
    /// `rotateOriginAround`.
    fn rotate_origin_around(&mut self, degrees: f32, pivot: [f64; 3]) {
        let rads = f64::from(degrees.to_radians());
        if rads != 0.0 {
            let p = self.room_origin;
            let (s, c) = rads.sin_cos();
            self.room_origin = [
                c * (p[0] - pivot[0]) - s * (p[2] - pivot[2]) + pivot[0],
                p[1],
                s * (p[0] - pivot[0]) + c * (p[2] - pivot[2]) + pivot[2],
            ];
        }
    }
    /// The right stick's snap turn (`MCVR` axis snap turning).
    fn snap_turn(&mut self, axis: f32) {
        let increment = settings::world_rotation_increment();
        if increment == 0.0 {
            // Smooth rotation: `worldRotationXSensitivity` 1, 10 degrees at a full push.
            // shortcut: per tick, where Vivecraft turns per frame.
            if axis.abs() > 0.1 {
                self.world_rotation = (self.world_rotation - 10.0 * axis) % 360.0;
            }
            return;
        }
        if !self.snapped && axis.abs() > 0.5 {
            self.world_rotation -= increment * axis.signum();
            self.world_rotation %= 360.0;
            self.snapped = true;
        } else if axis.abs() < 0.3 {
            self.snapped = false;
        }
    }
}

static SIM_LOOK: std::sync::Mutex<Option<(f32, f32)>> = std::sync::Mutex::new(None);

impl Game<'_> {
    /// Whether the port drives the player: VR shows the game and a player is in a world.
    pub(super) fn vr_running(&self) -> bool {
        // The server thread's own `Game` (`serving`) shares the process's VR state: never it.
        self.serving.is_none()
            && horizon_vr::poses().is_some()
            && self.player.is_some()
            && self.title.is_none()
    }

    /// `aim()` in VR: the main controller's ray (`getAim`, `AimDevice.CONTROLLER`).
    pub(super) fn vr_aim(&self) -> Option<([f64; 3], [f64; 3])> {
        if !self.vr_running() {
            return None;
        }
        let poses = horizon_vr::poses()?;
        let guard = vr_player();
        let vr = guard.as_ref().filter(|v| v.placed)?;
        let data = vr.data(poses);
        let d = data.controller_dir(0).normalize_or_zero();
        Some((
            data.controller_pos(0),
            [f64::from(d.x), f64::from(d.y), f64::from(d.z)],
        ))
    }

    /// `preTick`, before the player's physics: this tick's tracking with last tick's room, the
    /// snap turn, and the walking direction (`freemoveDirection`, the offhand controller).
    pub(super) fn vr_pre_tick(&mut self) {
        if !self.vr_running() {
            let mut guard = vr_player();
            if guard.is_some() {
                *guard = None;
                horizon_vr::set_room(None);
                horizon_vr::set_head_in_block(false);
                horizon_vr::set_eye_tint(None);
                super::vr_camera::vr_camera_reset();
            }
            drop(guard);
            // These trackers own transient input and pose state.  Clear it when tracking ends
            // so a desktop player cannot inherit a VR crawl, run input, grip, or radial menu.
            if let Some(player) = self.player.as_mut() {
                player.vr_crawl = false;
            }
            if self.serving.is_none() {
                super::vr_crawl::vr_crawl_stop();
            }
            super::vr_run::vr_run_reset();
            super::vr_climb_claws::vr_boots_reset();
            super::vr_radial::vr_radial_reset();
            // A simulated look can have been saved in `vr_pre_tick` but never consumed if
            // tracking stops before `vr_post_tick`.  Do not apply it to a later real-VR tick.
            *SIM_LOOK.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }
        let Some(poses) = horizon_vr::poses() else {
            return;
        };
        let Some(position) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let player_yaw = self.yaw;
        let joined = self.resident_entities.local_player.join();
        let world_scale = super::vr_settings::vr_settings().world_scale;
        let mut guard = vr_player();
        let vr = guard.get_or_insert_with(VrPlayer::new);
        vr.world_scale = world_scale;
        if !vr.placed || vr.joined != joined {
            vr.joined = joined;
            // The headset faces the player's yaw: the room turns by the difference.
            let room_yaw = yaw_of(poses.hmd.direction());
            vr.world_rotation = wrap_degrees(room_yaw - player_yaw);
            vr.room_origin = [position.x, position.y, position.z];
            let data = vr.data(poses);
            snap_room_origin(vr, &data, [position.x, position.y, position.z]);
            vr.placed = true;
            log::info!(
                "Horizon VR: room placed at {:?}, rotation {:.1}",
                vr.room_origin,
                vr.world_rotation
            );
        }
        let data = vr.data(poses);
        vr.pre = Some(data);
        vr.before = Some([position.x, position.y, position.z]);
        vr.snap_turn(poses.turn);
        vr.vehicle_yaw = self.riding.is_some().then_some(player_yaw);
        drop(guard);
        // `HORIZON_VR=sim`: the simulated hands follow the desktop look, so the look is kept
        // across the tick instead of following them (it would feed back into itself).
        if horizon_vr::sim_mode() {
            *SIM_LOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some((self.yaw, self.pitch));
        }
        // Walking goes where the offhand controller points (the yaw the physics reads).
        // `freemoveDirection` by `vrFreeMoveMode`.
        if !self.riding.is_some() {
            let s = super::vr_settings::vr_settings();
            self.yaw = match if s.seated { super::vr_settings::FreeMove::Hmd } else { s.free_move } {
                super::vr_settings::FreeMove::Controller => yaw_of(data.controller_dir(1)),
                super::vr_settings::FreeMove::Hmd => yaw_of(data.room.dir_to_world(poses.hmd.direction())),
                super::vr_settings::FreeMove::RunInPlace => super::vr_run::vr_run_yaw().unwrap_or(self.yaw),
            };
        }
    }

    /// After the player's physics: the room follows the game's moves, the player follows the
    /// headset (`doPlayerMoveInRoom`), the trackers run, the snap turn turns the room about the
    /// head (`postTick`) and the player looks where the main controller aims.
    pub(super) fn vr_post_tick(&mut self) -> horizon_client::Result<()> {
        // `VehicleTracker` (vehicleRotation on): the room turns with the vehicle's turn this tick.
        if let Some(vr) = vr_player().as_mut()
            && let Some(before) = vr.vehicle_yaw.take()
            && self.riding.is_some()
        {
            vr.world_rotation = (vr.world_rotation - wrap_degrees(self.yaw - before)) % 360.0;
        }
        if let Some((yaw, pitch)) = SIM_LOOK.lock().unwrap_or_else(|e| e.into_inner()).take() {
            self.yaw = yaw;
            self.pitch = pitch;
        }
        let Some(poses) = horizon_vr::poses() else {
            return Ok(());
        };
        let Some(position) = self.player.as_ref().map(|p| p.position) else {
            return Ok(());
        };
        let riding = self.riding.is_some();
        let _ = position;
        {
            let position = self.player.as_ref().map(|p| p.position).unwrap_or(position);
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            // Hanging from a ladder or vine: no gravity (`setNoGravity`), the hands move the player.
            if vr.trackers.climb.hanging
                && let Some(before) = vr.before
                && let Some(p) = self.player.as_mut()
            {
                p.position.x = before[0];
                p.position.y = before[1];
                p.position.z = before[2];
                p.velocity.y = 0.0;
                p.fall_distance = 0.0;
                vr.before = None;
            }
            // `setPos`/`move`: the room moves with the player the game moved.
            if let Some(before) = vr.before.take()
                && !riding
            {
                let now = [position.x, position.y, position.z];
                for i in 0..3 {
                    vr.room_origin[i] += now[i] - before[i];
                }
                // A teleport or respawn: the room lands under the player.
                let jump: f64 = (0..3).map(|i| (now[i] - before[i]).powi(2)).sum();
                if jump > 64.0 {
                    let data = vr.data(poses);
                    snap_room_origin(vr, &data, now);
                }
            }
        }
        let seated = super::vr_settings::vr_settings().seated;
        if seated {
            // Seated: the room stays on the player each tick (`snapRoomOriginToPlayerEntity`),
            // the head at standing eye height.
            if let (Some(p), Some(vr)) = (self.player.as_ref().map(|p| p.position), vr_player().as_mut()) {
                let data = vr.data(poses);
                snap_room_origin(vr, &data, [p.x, p.y, p.z]);
                // shortcut: a fixed 1.62 m eye; Vivecraft calibrates the seated height.
                vr.room_origin[1] += f64::from(1.62 - poses.hmd.position[1] * vr.world_scale);
            }
        } else {
            self.vr_move_in_room(poses)?;
        }
        self.vr_tick_trackers(poses)?;
        let mut guard = vr_player();
        let Some(vr) = guard.as_mut() else {
            return Ok(());
        };
        // `postTick`: turn the room about the head by this tick's snap turn.
        if let Some(pre) = vr.pre {
            let start = pre.room.rotation.to_degrees();
            let end = vr.world_rotation;
            let pivot = pre.new_head_pivot(vr.room_origin, vr.world_scale);
            vr.rotate_origin_around(-end + start, pivot);
        }
        let post = vr.data(poses);
        horizon_vr::set_room(Some(post.room));
        let head = post.room.to_world(poses.hmd.translation()).map(|v| v.floor() as i32);
        horizon_vr::set_head_in_block(self.state_suffocating(self.world.block(head[0], head[1], head[2])));
        // `crossVec` for the look override: the aim's reach along the main controller.
        let reach = self.block_reach;
        let d = post.controller_dir(0).normalize_or_zero();
        let from = post.controller_pos(0);
        vr.cross_vec = Some(std::array::from_fn(|i| {
            from[i] + f64::from([d.x, d.y, d.z][i]) * reach
        }));
        let cross = vr.cross_vec;
        drop(guard);
        if horizon_vr::sim_mode() {
            return Ok(());
        }
        self.vr_look_override(post, cross);
        Ok(())
    }

    /// `doPermanentLookOverride`: the player looks at where the main controller aims, else
    /// where the headset looks; swimming and gliding look with the headset.
    fn vr_look_override(&mut self, data: VrData, cross: Option<[f64; 3]>) {
        let Some(eye) = self.eye_position() else {
            return;
        };
        let (gliding, swimming) = self
            .player
            .as_ref()
            .map_or((false, false), |p| (p.fall_flying, p.swimming));
        let (yaw, pitch) = match cross {
            Some(cross) if !gliding && !swimming => {
                // Backwards: from the crosshair to the eyes.
                let d = [eye[0] - cross[0], eye[1] - cross[1], eye[2] - cross[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len == 0.0 {
                    return;
                }
                let pitch = (d[1] / len).clamp(-1.0, 1.0).asin().to_degrees() as f32;
                let yaw = d[0].atan2(-d[2]).to_degrees() as f32;
                (yaw, pitch)
            }
            _ => {
                let d = data.hmd_dir();
                (yaw_of(d), -pitch_of(d))
            }
        };
        let turn = wrap_degrees(yaw - self.yaw);
        self.turn_player(
            f64::from(turn / 0.15),
            f64::from((pitch - self.pitch) / 0.15),
        );
    }

    /// `doPlayerMoveInRoom`: the player's feet go under the head's pivot where they fit, step up
    /// a block in the way, and stay where the game holds them (sneaking, jumping, climbing).
    fn vr_move_in_room(&mut self, poses: Poses) -> horizon_client::Result<()> {
        let (pivot, climbing) = {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            if vr.room_scale_movement_delay > 0 {
                vr.room_scale_movement_delay -= 1;
                return Ok(());
            }
            let Some(pre) = vr.pre else {
                return Ok(());
            };
            if vr.trackers.jump.is_jumping() {
                return Ok(());
            }
            let _ = poses;
            (
                pre.new_head_pivot(vr.room_origin, vr.world_scale),
                vr.trackers.climb.is_grabbing_ladder(),
            )
        };
        if climbing || self.sleeping.is_some() || self.riding.is_some() {
            return Ok(());
        }
        let Some(player) = self.player.as_ref() else {
            return Ok(());
        };
        if player.shift_key_down || player.spectator || self.screen == Screen::Dead {
            return Ok(());
        }
        let half = f64::from(player.dimensions.width) / 2.0;
        let height = f64::from(player.dimensions.height);
        let fall = player.fall_distance;
        let (x, y, z) = (pivot[0], player.position.y, pivot[2]);
        let bb = Aabb::new(
            WVec3::new(x - half, y, z - half),
            WVec3::new(x + half, y + height, z + half),
        );
        if self.vr_no_collision(bb)? {
            let player = self.player.as_mut().expect("checked");
            player.position.x = x;
            player.position.z = z;
            player.fall_distance = 0.0;
            return Ok(());
        }
        if !(settings::walk_up_blocks() || climbing) || fall != 0.0 {
            return Ok(());
        }
        // Colliding: is the player well inside a block? Then try up to a block higher.
        let shrink =
            half - f64::from(self.player.as_ref().expect("checked").dimensions.width) * 0.45;
        let climb = Aabb::new(
            WVec3::new(x - shrink, bb.min(Axis::Y), z - shrink),
            WVec3::new(x + shrink, bb.max(Axis::Y), z + shrink),
        );
        if self.vr_no_collision(climb)? {
            return Ok(());
        }
        let extra = if self.player.as_ref().is_some_and(|p| p.on_climbable) {
            6
        } else {
            0
        };
        let mut up = bb;
        for i in 0..=(10 + extra) {
            up = up.move_by(WVec3::new(0.0, 0.1, 0.0));
            if self.vr_no_collision(up)? {
                let player = self.player.as_mut().expect("checked");
                player.position.x = x;
                player.position.y = up.min(Axis::Y);
                player.position.z = z;
                player.fall_distance = 0.0;
                if let Some(vr) = vr_player().as_mut() {
                    vr.room_origin[1] += 0.1 * f64::from(i + 1);
                }
                break;
            }
        }
        Ok(())
    }

    /// `Level.noCollision(player, bounds)` for the local player.
    pub(super) fn vr_no_collision(&mut self, bounds: Aabb) -> horizon_client::Result<bool> {
        let Some(player) = self.player.as_mut() else {
            return Ok(true);
        };
        let mut world = horizon_client::motion::MotionWorld {
            entities: &[],
            player,
            scene: &mut self.world,
            assets: &self.pack.assets,
            collision_binding: self.collision,
            resources: self.motion,
            health: &self.health_resources,
            loop_context: &self.loop_context,
            damage_hooks: &self.damage_hooks,
            tag_items: None,
            block_registries: None,
        };
        world.no_player_collision(bounds)
    }
}

/// `snapRoomOriginToPlayerEntity`: the room under `at` (the player's feet), the head's pivot
/// over it.
fn snap_room_origin(vr: &mut VrPlayer, data: &VrData, at: [f64; 3]) {
    let pivot = data.new_head_pivot(vr.room_origin, vr.world_scale);
    let cam = [pivot[0] - vr.room_origin[0], pivot[2] - vr.room_origin[2]];
    vr.room_origin = [at[0] - cam[0], at[1], at[2] - cam[1]];
}
