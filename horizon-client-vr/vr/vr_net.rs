//! Vivecraft's `ClientNetworking`, ported from VivecraftMod (LGPL-3.0,
//! `client/network/ClientNetworking.java`, `common/network/**`): on a server with Vivecraft's plugin
//! (channel `vivecraft:data`) the game tells it where the headset and hands are (so other players see
//! a VR player and the server aims from the hand), crawls, climbs, climbey jumps, teleports and bow
//! draws, and hears what the server allows and where other VR players' heads and hands are.
use super::Game;
use super::vr_player::vr_player;
use horizon_protocol::packets::{configuration::CustomPayload, play::Serverbound as S};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

pub(crate) const CHANNEL: &str = "vivecraft:data";
/// `NetworkVersion`: protocol numbers (`ordinal - 1`). Horizon speaks up to `OPTION_TOGGLE`'s
/// successors it needs: `ROOMSCALE_ATTACK_PACKET` (10) is the newest, `NEW_NETWORKING` (0) the oldest.
const MAX_PROTOCOL: i32 = 10;
const MIN_PROTOCOL: i32 = 0;
const LEGACY: i32 = -1;
/// `PayloadIdentifier` ordinals.
mod id {
    pub const VERSION: u8 = 0;
    pub const REQUESTDATA: u8 = 1;
    pub const HEADDATA: u8 = 2;
    pub const CONTROLLER0DATA: u8 = 3;
    pub const CONTROLLER1DATA: u8 = 4;
    pub const WORLDSCALE: u8 = 5;
    pub const DRAW: u8 = 6;
    pub const UBERPACKET: u8 = 8;
    pub const TELEPORT: u8 = 9;
    pub const CLIMBING: u8 = 10;
    pub const HEIGHT: u8 = 12;
    pub const ACTIVEHAND: u8 = 13;
    pub const CRAWL: u8 = 14;
    pub const NETWORK_VERSION: u8 = 15;
    pub const IS_VR_ACTIVE: u8 = 17;
    pub const VR_PLAYER_STATE: u8 = 18;
    pub const JUMPING: u8 = 27;
}

/// A device's pose as Vivecraft sends it: position from the player's feet, orientation in the world.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NetPose {
    pub position: glam::Vec3,
    pub orientation: glam::Quat,
}

/// `VrPlayerState`, the parts Horizon draws (full-body trackers are read past).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NetVrPlayer {
    pub seated: bool,
    pub left_handed: bool,
    pub hmd: NetPose,
    pub main_hand: NetPose,
    pub off_hand: NetPose,
    pub world_scale: f32,
    pub height_scale: f32,
    /// `FBTMode.ARMS_LEGS` and up: the waist, right foot and left foot trackers.
    pub feet: Option<[NetPose; 3]>,
}

#[derive(Default)]
pub(crate) struct Net {
    pub has_vivecraft: bool,
    pub wants_data: bool,
    pub version: i32,
    pub allows_crawl: bool,
    pub allows_climbey: bool,
    pub supports_direct_teleport: bool,
    pub allows_direct_teleport: bool,
    /// `climbTracker.serverBlockmode` (0 off, 1 only these, 2 all but these) and its blocks.
    pub climb_mode: u8,
    pub climb_blocks: Vec<String>,
    /// `ClientVRPlayers`: other VR players by UUID.
    pub players: HashMap<u128, NetVrPlayer>,
    world_scale_last: f32,
    height_last: f32,
    crawl_last: bool,
    body_part_last: u8,
    draw_last: f32,
}

static NET: Mutex<Option<Net>> = Mutex::new(None);

pub(crate) fn vr_net() -> MutexGuard<'static, Option<Net>> {
    NET.lock().unwrap_or_else(|e| e.into_inner())
}

fn with<T: Default>(f: impl FnOnce(&Net) -> T) -> T {
    vr_net().as_ref().map(f).unwrap_or_default()
}

/// `SERVER_ALLOWS_CRAWLING`.
pub(crate) fn server_allows_crawling() -> bool {
    with(|n| n.allows_crawl)
}
/// `SERVER_ALLOWS_CLIMBEY`.
pub(crate) fn server_allows_climbey() -> bool {
    with(|n| n.allows_climbey)
}
/// The server takes `TeleportPayloadC2S` (`SERVER_SUPPORTS_DIRECT_TELEPORT && ALLOWS`).
pub(crate) fn server_direct_teleport() -> bool {
    with(|n| n.supports_direct_teleport && n.allows_direct_teleport)
}
/// The server forbids teleporting (`setTeleportOverride(false)`).
pub(crate) fn server_forbids_teleport() -> bool {
    with(|n| n.supports_direct_teleport && !n.allows_direct_teleport)
}
/// `climbTracker.blocklist` with `serverBlockmode`: whether claws may hold `block` (an id).
pub(crate) fn server_claws_block(block: &str) -> bool {
    with(|n| {
        let listed = n.climb_blocks.iter().any(|b| b.strip_prefix("minecraft:").unwrap_or(b) == block.strip_prefix("minecraft:").unwrap_or(block));
        match n.climb_mode {
            1 => listed,
            2 => !listed,
            _ => true,
        }
    })
}
/// The body's `ModelPart` angles (x, y, z) from the waist tracker's world orientation: the turn
/// from the body's facing, then into the model's axes (x and y flipped), as `Rz * Ry * Rx`.
/// Positive x leans forward, as crouching does.
fn waist_angles(q: glam::Quat, body_yaw: f32) -> [f32; 3] {
    let facing = glam::Quat::from_rotation_y(std::f32::consts::PI - body_yaw.to_radians());
    let flip = glam::Quat::from_rotation_z(std::f32::consts::PI);
    let local = flip * (facing.inverse() * q) * flip.inverse();
    let (z, y, x) = local.to_euler(glam::EulerRot::ZYX);
    [x, y, z]
}

/// Another VR player's tracking, by UUID.
pub(crate) fn vr_net_player(uuid: u128) -> Option<NetVrPlayer> {
    vr_net().as_ref()?.players.get(&uuid).copied()
}

// FriendlyByteBuf, big-endian.
fn put_f32(b: &mut Vec<u8>, v: f32) {
    b.extend_from_slice(&v.to_be_bytes());
}
fn put_pose(b: &mut Vec<u8>, p: NetPose) {
    for v in p.position.to_array() {
        put_f32(b, v);
    }
    let q = p.orientation;
    for v in [q.w, q.x, q.y, q.z] {
        put_f32(b, v);
    }
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn bool(&mut self) -> Option<bool> {
        self.u8().map(|b| b != 0)
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn uuid(&mut self) -> Option<u128> {
        Some(u128::from_be_bytes(self.take(16)?.try_into().ok()?))
    }
    fn varint(&mut self) -> Option<usize> {
        let mut v = 0usize;
        for i in 0..5 {
            let b = self.u8()?;
            v |= usize::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    fn utf(&mut self) -> Option<String> {
        let n = self.varint()?;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn pose(&mut self) -> Option<NetPose> {
        let position = glam::Vec3::new(self.f32()?, self.f32()?, self.f32()?);
        let w = self.f32()?;
        let q = glam::Quat::from_xyzw(self.f32()?, self.f32()?, self.f32()?, w);
        // Untrusted: a zero or non-finite quaternion would normalize to NaN.
        if !position.is_finite() || !q.is_finite() || q.length_squared() < 1e-6 {
            return None;
        }
        Some(NetPose { position, orientation: q.normalize() })
    }
    /// `VrPlayerState.deserialize(buffer, bytes_after)`.
    fn state(&mut self, bytes_after: usize) -> Option<NetVrPlayer> {
        let seated = self.bool()?;
        let hmd = self.pose()?;
        let left_handed = self.bool()?;
        let main_hand = self.pose()?;
        let _legacy = self.bool()?;
        let off_hand = self.pose()?;
        let fbt = if self.0.len() > bytes_after { self.u8()? } else { 0 };
        // FBTMode: ARMS_ONLY, ARMS_LEGS (waist, feet), WITH_JOINTS (and knees, elbows).
        // Waist, right foot, left foot, then (WITH_JOINTS) the knees and elbows.
        let mut feet = None;
        if fbt >= 1 {
            feet = Some([self.pose()?, self.pose()?, self.pose()?]);
        }
        if fbt >= 2 {
            for _ in 0..4 {
                self.pose()?;
            }
        }
        Some(NetVrPlayer { seated, left_handed, hmd, main_hand, off_hand, world_scale: 1.0, height_scale: 1.0, feet })
    }
}

/// Reads one `vivecraft:data` payload into `net`. Returns a chat line to show, if any.
fn receive(net: &mut Net, data: &[u8]) -> Option<String> {
    let mut r = Reader(data);
    match r.u8()? {
        id::VERSION => {
            net.has_vivecraft = true;
            let v = r.utf().unwrap_or_default();
            return Some(format!("Vivecraft server plugin: {v}"));
        }
        id::REQUESTDATA => net.wants_data = true,
        id::CLIMBING => {
            // Parse it all first, so a truncated packet leaves the old rules as they were.
            let allows = r.bool()?;
            let mode = r.u8().unwrap_or(0);
            let mut blocks = Vec::new();
            while !r.0.is_empty() {
                blocks.push(r.utf()?);
            }
            (net.allows_climbey, net.climb_mode, net.climb_blocks) = (allows, mode, blocks);
        }
        id::TELEPORT => {
            net.supports_direct_teleport = true;
            net.allows_direct_teleport = r.bool().unwrap_or(true);
        }
        id::UBERPACKET => {
            let uuid = r.uuid()?;
            let mut p = r.state(8)?;
            p.world_scale = r.f32()?;
            p.height_scale = r.f32()?;
            if !(p.world_scale.is_finite() && p.height_scale.is_finite()) {
                return None;
            }
            // shortcut: caps players a server never marks inactive, a TTL if servers leak many.
            if net.players.len() >= 512 && !net.players.contains_key(&uuid) {
                return None;
            }
            net.players.insert(uuid, p);
        }
        id::CRAWL => net.allows_crawl = r.bool().unwrap_or(true),
        id::NETWORK_VERSION => net.version = i32::from(r.u8()? as i8),
        id::IS_VR_ACTIVE => {
            let vr = r.bool()?;
            let uuid = r.uuid()?;
            if !vr {
                net.players.remove(&uuid);
            }
        }
        // Settings overrides, VR switching, dual wielding, haptics, damage directions and the
        // rest change nothing Horizon draws or decides.
        _ => {}
    }
    None
}

impl Game<'_> {
    fn vr_net_send(&self, data: Vec<u8>) {
        self.send(S::CustomPayload(CustomPayload { channel: CHANNEL.into(), data }));
    }

    /// `resetServerSettings` and `sendVersionInfo`, on joining a server and on each respawn:
    /// the channel is announced (`minecraft:register`, which plugin servers wait for) and the
    /// version sent, VR or not, so the server knows a Vivecraft client is here.
    pub(crate) fn vr_net_join(&mut self) {
        if self.remote.is_none() || self.serving.is_some() {
            return;
        }
        *vr_net() = Some(Net { version: LEGACY, allows_direct_teleport: true, ..Default::default() });
        self.send(S::CustomPayload(CustomPayload {
            channel: "minecraft:register".into(),
            data: CHANNEL.as_bytes().to_vec(),
        }));
        let vr = if horizon_vr::requested() { "VR" } else { "NONVR" };
        let mut b = vec![id::VERSION];
        b.extend_from_slice(format!("Vivecraft-26.3-horizon-1.3.0 {vr}\n{MAX_PROTOCOL}\n{MIN_PROTOCOL}").as_bytes());
        self.vr_net_send(b);
    }

    /// A `vivecraft:data` payload from the server.
    pub(crate) fn vr_net_receive(&mut self, data: &[u8]) {
        let line = vr_net().as_mut().and_then(|n| receive(n, data));
        if let Some(line) = line {
            log::info!("Horizon VR: {line}");
        }
    }

    fn vr_net_ready(&self) -> bool {
        self.remote.is_some() && self.serving.is_none() && with(|n| n.has_vivecraft)
    }

    /// `sendVRPlayerPositions` (each tick after the player's physics) with the crawl's change.
    pub(crate) fn vr_net_tick(&mut self) {
        if !self.vr_net_ready() {
            return;
        }
        let Some(poses) = horizon_vr::poses() else {
            return;
        };
        let Some(feet) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let crawling = self.player.as_ref().is_some_and(|p| p.vr_crawl);
        let (data, scale, climbing) = {
            let guard = vr_player();
            let Some(vr) = guard.as_ref() else {
                return;
            };
            (vr.data(poses), vr.world_scale, vr.trackers.climb.is_grabbing_ladder())
        };
        // `ClimbTracker`: each tick held on, no fall damage and no kick for flying.
        if climbing {
            self.vr_net_climbing();
        }
        let settings = super::vr_settings::vr_settings();
        let (seated, left) = (settings.seated, horizon_vr::reversed());
        let rel = |p: [f64; 3]| {
            glam::Vec3::new((p[0] - feet.x) as f32, (p[1] - feet.y) as f32, (p[2] - feet.z) as f32)
        };
        let turn = data.room.turn();
        let hmd = NetPose { position: rel(data.hmd_pos()), orientation: (turn * poses.hmd.rotation()).normalize() };
        let hand = |c: usize| NetPose {
            position: rel(data.controller_pos(c)),
            orientation: (turn * poses.aim[c].map_or(poses.hmd.rotation(), |p| p.rotation())).normalize(),
        };
        let (main, off) = (hand(0), hand(1));
        let body = super::vr_osc::osc_body(poses.hmd).map(|parts| {
            parts.map(|(at, q)| NetPose { position: rel(data.room.to_world(at)), orientation: (turn * q).normalize() })
        });
        // `AutoCalibration.getPlayerHeight() / DEFAULT_HEIGHT` (1.52 m eyes): the headset's height.
        let height = (data.hmd_height() / 1.52).clamp(0.5, 2.0);
        let mut out = Vec::new();
        {
            let mut guard = vr_net();
            let Some(net) = guard.as_mut() else {
                return;
            };
            if net.wants_data {
                if scale != net.world_scale_last {
                    let mut b = vec![id::WORLDSCALE];
                    put_f32(&mut b, scale);
                    out.push(b);
                    net.world_scale_last = scale;
                }
                // Sent rounded, so standing still doesn't send it every tick.
                let h = (height * 100.0).round() / 100.0;
                if h != net.height_last {
                    let mut b = vec![id::HEIGHT];
                    put_f32(&mut b, h);
                    out.push(b);
                    net.height_last = h;
                }
                if net.version != LEGACY {
                    let mut b = vec![id::VR_PLAYER_STATE, u8::from(seated)];
                    put_pose(&mut b, hmd);
                    b.push(u8::from(left));
                    put_pose(&mut b, main);
                    b.push(u8::from(left));
                    put_pose(&mut b, off);
                    // `FBT` (1) and newer: `FBTMode.ARMS_LEGS`, the waist and the feet.
                    if net.version >= 1 && let Some(body) = &body {
                        b.push(1);
                        for &pose in body {
                            put_pose(&mut b, pose);
                        }
                    }
                    out.push(b);
                } else {
                    for (k, pose) in [(id::CONTROLLER0DATA, main), (id::CONTROLLER1DATA, off)] {
                        let mut b = vec![k, u8::from(left)];
                        put_pose(&mut b, pose);
                        out.push(b);
                    }
                    let mut b = vec![id::HEADDATA, u8::from(seated)];
                    put_pose(&mut b, hmd);
                    out.push(b);
                }
            }
            if net.allows_crawl && crawling != net.crawl_last {
                out.push(vec![id::CRAWL, u8::from(crawling)]);
                net.crawl_last = crawling;
            }
        }
        for b in out {
            self.vr_net_send(b);
        }
    }

    /// `ClimbingPayloadC2S`: the claws pulled the player (no fall damage, no kick for flying).
    pub(crate) fn vr_net_climbing(&self) {
        if self.vr_net_ready() && server_allows_climbey() {
            self.vr_net_send(vec![id::CLIMBING]);
        }
    }

    /// `JumpingPayloadC2S`: a climbey jump.
    pub(crate) fn vr_net_jumping(&self) {
        // `CLIMBEY_JUMP` (9) and newer servers know it.
        if self.vr_net_ready() && server_allows_climbey() && with(|n| n.version >= 9) {
            self.vr_net_send(vec![id::JUMPING]);
        }
    }

    /// `TeleportPayloadC2S`: the server moves the player there.
    pub(crate) fn vr_net_teleport(&self, to: [f64; 3]) {
        let mut b = vec![id::TELEPORT];
        for v in to {
            put_f32(&mut b, v as f32);
        }
        self.vr_net_send(b);
    }

    /// `BowModule`'s `DrawPayloadC2S` and `ActiveBodyPartPayloadC2S`: the draw (0 none) and the hand
    /// the arrow is in (0 main, 1 offhand), so the server shoots from the hands.
    pub(crate) fn vr_net_bow(&self, draw: f32, arrow_hand: usize) {
        if !self.vr_net_ready() || !with(|n| n.wants_data) {
            return;
        }
        let part = if draw > 0.0 { arrow_hand as u8 } else { 0 };
        let (send_draw, send_part, version) = {
            let mut guard = vr_net();
            let Some(net) = guard.as_mut() else {
                return;
            };
            let d = (draw != net.draw_last).then(|| net.draw_last = draw).is_some();
            let p = (part != net.body_part_last).then(|| net.body_part_last = part).is_some();
            (d, p, net.version)
        };
        if send_draw {
            let mut b = vec![id::DRAW];
            put_f32(&mut b, draw);
            self.vr_net_send(b);
        }
        if send_part {
            let mut b = vec![id::ACTIVEHAND, part];
            // `HEAD_AIM` (3) and newer read the aim flag.
            if version >= 3 {
                b.push(1);
            }
            self.vr_net_send(b);
        }
    }
}

/// `VRPlayerModel_WithArms`'s arm angles (`xRot`, `yRot`, `zRot`) so an arm hanging from its
/// shoulder points at `hand`: positions in model space (1/16 block, y down, facing -z).
fn arm_angles(shoulder: glam::Vec3, hand: glam::Vec3) -> [f32; 3] {
    let d = (hand - shoulder).normalize_or(glam::Vec3::Y);
    let h = (d.x * d.x + d.z * d.z).sqrt();
    // `ModelPart` turns Z, then Y, then X: Ry(y) * Rx(x) * down = d.
    // Straight down has no heading: no twist.
    let y = if h < 1e-4 { 0.0 } else { (-d.x).atan2(-d.z) };
    [-h.atan2(d.y), y, 0.0]
}

/// A world offset from a player's feet in its model's space (`LivingEntityRenderer`: turned by
/// `180 - bodyYaw`, scaled -1, -1, 1, its origin 1.501 blocks up), in blocks.
fn to_model(rel: glam::Vec3, body_yaw: f32) -> glam::Vec3 {
    let v = glam::Quat::from_rotation_y(-(180.0 - body_yaw).to_radians()) * (rel - glam::Vec3::new(0.0, 1.501, 0.0));
    glam::Vec3::new(-v.x, -v.y, v.z)
}

impl Game<'_> {
    /// Another VR player's arm angles ([right, left], as `ArmPose::Custom` bits), from the server's
    /// tracking of it (`ClientVRPlayers`).
    pub(crate) fn vr_net_arm_angles(&self, entity_id: u32, m: &horizon_client::mobs::Mob) -> Option<[[u32; 3]; 2]> {
        let p = vr_net_player(self.remote_player_uuid(entity_id)?)?;
        let (right, left) = if p.left_handed { (p.off_hand, p.main_hand) } else { (p.main_hand, p.off_hand) };
        // `ModelPart` pivots: the right arm at (-5, 2, 0), the left at (5, 2, 0) pixels.
        let arm = |hand: NetPose, x: f32| {
            arm_angles(glam::Vec3::new(x, 2.0, 0.0) / 16.0, to_model(hand.position, m.body_yaw)).map(f32::to_bits)
        };
        Some([arm(right, -5.0), arm(left, 5.0)])
    }

    /// `VRPlayerModel_WithArmsLegs`: the body turns, leans and tilts with the waist tracker, the
    /// hips follow it sideways and forward, and the legs reach for the foot trackers. `ModelPart`
    /// pivots: the right leg at (-1.9, 12, 0), the left at (1.9, 12, 0) pixels.
    // shortcut: knees and elbows (`FBTMode.WITH_JOINTS`) aren't bent; the vanilla model has no lower legs to bend.
    pub(crate) fn vr_net_leg_angles(&self, entity_id: u32, m: &horizon_client::mobs::Mob) -> Option<([[f32; 3]; 3], [f32; 3])> {
        let [waist, right, left] = vr_net_player(self.remote_player_uuid(entity_id)?)?.feet?;
        // How far the hips moved from under the shoulders, in pixels, kept within the model's reach.
        let w = to_model(waist.position, m.body_yaw) * 16.0;
        let hips = [w.x.clamp(-4.0, 4.0), 0.0, w.z.clamp(-4.0, 4.0)];
        let leg = |foot: NetPose, x: f32| {
            let pivot = glam::Vec3::new(x + hips[0], 12.0, hips[2]) / 16.0;
            arm_angles(pivot, to_model(foot.position, m.body_yaw))
        };
        Some(([leg(right, -1.9), leg(left, 1.9), waist_angles(waist.orientation, m.body_yaw)], hips))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arms_point_at_hands() {
        // Hanging down: no turn. Straight ahead (-z in the model): -90 degrees about x.
        let down = arm_angles(glam::Vec3::ZERO, glam::Vec3::Y);
        assert!(down[0].abs() < 1e-5 && down[1].abs() < 1e-5);
        let ahead = arm_angles(glam::Vec3::ZERO, glam::Vec3::NEG_Z);
        assert!((ahead[0] + std::f32::consts::FRAC_PI_2).abs() < 1e-5 && ahead[1].abs() < 1e-5);
        // A player facing south (body yaw 0, +z in the world): a hand in front is ahead in the model.
        let m = to_model(glam::Vec3::new(0.0, 1.375, 1.0), 0.0);
        assert!(m.z < -0.9, "{m}");
    }

    #[test]
    fn legs_point_at_feet() {
        // A right foot straight below the hip (facing south, the right is world -x) leaves the leg hanging.
        let hip = glam::Vec3::new(-1.9, 12.0, 0.0) / 16.0;
        let a = arm_angles(hip, to_model(glam::Vec3::new(-1.9 / 16.0, 0.0, 0.0), 0.0));
        assert!(a.iter().all(|v| v.abs() < 1e-3), "{a:?}");
        // Facing south (body yaw 0) is a turn of 180 degrees about y in the world.
        let south = glam::Quat::from_rotation_y(std::f32::consts::PI);
        let a = waist_angles(south, 0.0);
        assert!(a.iter().all(|v| v.abs() < 1e-4), "{a:?}");
        // A waist pitched 30 degrees down leans the body 30 degrees forward.
        let a = waist_angles(south * glam::Quat::from_rotation_x(-30f32.to_radians()), 0.0);
        assert!((a[0] - 30f32.to_radians()).abs() < 1e-4 && a[1].abs() < 1e-4, "{a:?}");
    }

    #[test]
    fn reads_server_packets() {
        let mut n = Net::default();
        // VERSION with a writeUtf string.
        assert!(receive(&mut n, &[id::VERSION, 3, b'1', b'.', b'2']).is_some());
        assert!(n.has_vivecraft);
        receive(&mut n, &[id::CRAWL]);
        assert!(n.allows_crawl, "an old server's empty crawl packet allows it");
        receive(&mut n, &[id::CLIMBING, 1, 1, 5, b's', b't', b'o', b'n', b'e']);
        assert!(n.allows_climbey && n.climb_mode == 1 && n.climb_blocks == ["stone"]);
        // An uber packet: uuid, state (no FBT byte: 8 bytes after), world and height scale.
        let mut b = vec![id::UBERPACKET];
        b.extend_from_slice(&7u128.to_be_bytes());
        let pose = NetPose { position: glam::Vec3::new(0.0, 1.6, 0.0), orientation: glam::Quat::IDENTITY };
        b.push(0);
        put_pose(&mut b, pose);
        b.push(1);
        put_pose(&mut b, pose);
        b.push(1);
        put_pose(&mut b, pose);
        put_f32(&mut b, 2.0);
        put_f32(&mut b, 1.0);
        receive(&mut n, &b);
        let p = n.players[&7];
        assert!(p.left_handed && p.world_scale == 2.0 && (p.hmd.position.y - 1.6).abs() < 1e-6);
        // A truncated climbing packet keeps the old rules.
        receive(&mut n, &[id::CLIMBING, 0, 2, 5, b'a']);
        assert!(n.allows_climbey && n.climb_mode == 1 && n.climb_blocks == ["stone"]);
        // A zero quaternion (it would normalize to NaN) drops the packet.
        let zero = NetPose { orientation: glam::Quat::from_xyzw(0.0, 0.0, 0.0, 0.0), ..pose };
        let mut bad = vec![id::UBERPACKET];
        bad.extend_from_slice(&8u128.to_be_bytes());
        for (hand, p) in [(0, zero), (1, pose), (1, pose)] {
            bad.push(hand);
            put_pose(&mut bad, p);
        }
        put_f32(&mut bad, 1.0);
        put_f32(&mut bad, 1.0);
        receive(&mut n, &bad);
        assert!(!n.players.contains_key(&8));
    }
}
