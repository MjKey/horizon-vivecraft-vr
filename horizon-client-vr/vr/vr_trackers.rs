//! Vivecraft's trackers, ported from VivecraftMod (LGPL-3.0,
//! https://github.com/Vivecraft/VivecraftMod, `client_vr/gameplay/trackers/`): the roomscale
//! actions read from the tracked head and hands each tick.
use super::vr_player::{VrData, vr_player, yaw_of};
use super::{Game, Hit, Screen};
use glam::{Quat, Vec3};
use horizon_client::first_person::Hand;
use horizon_core::geometry::{Aabb, Vec3 as WVec3};
use horizon_vr::Poses;
use std::collections::VecDeque;
use std::time::Instant;

/// `AutoCalibration.DEFAULT_HEIGHT`: the player's standing height the thresholds are from.
const PLAYER_HEIGHT: f32 = 1.52;
/// `VRSettings.jumpThreshold`, `sneakThreshold`.
const JUMP_THRESHOLD: f32 = 0.05;
const SNEAK_THRESHOLD: f32 = 0.4;

/// `Vector3fHistory`: timed samples, the last 450.
#[derive(Default)]
pub(super) struct History {
    data: VecDeque<(Vec3, Instant)>,
}

impl History {
    const CAPACITY: usize = 450;
    pub fn add(&mut self, v: Vec3) {
        self.data.push_back((v, Instant::now()));
        if self.data.len() > Self::CAPACITY {
            self.data.pop_front();
        }
    }
    pub fn clear(&mut self) {
        self.data.clear();
    }
    pub fn latest(&self) -> Vec3 {
        self.data.back().map_or(Vec3::ZERO, |e| e.0)
    }
    fn recent(&self, seconds: f64) -> impl Iterator<Item = &(Vec3, Instant)> {
        let now = Instant::now();
        self.data
            .iter()
            .rev()
            .take_while(move |e| now.duration_since(e.1).as_secs_f64() <= seconds)
    }
    /// `netMovement`: newest minus oldest within `seconds`.
    pub fn net_movement(&self, seconds: f64) -> Vec3 {
        let mut it = self.recent(seconds);
        let Some(last) = it.next() else {
            return Vec3::ZERO;
        };
        match it.last() {
            Some(first) => last.0 - first.0,
            None => Vec3::ZERO,
        }
    }
    /// `averageSpeed`: the mean speed between samples within `seconds`, m/s.
    pub fn average_speed(&self, seconds: f64) -> f32 {
        let mut total = 0.0;
        let mut count = 0;
        let mut last: Option<&(Vec3, Instant)> = None;
        for current in self.recent(seconds) {
            if let Some(last) = last {
                let dt = last.1.duration_since(current.1).as_secs_f32();
                if dt > 0.0 {
                    total += last.0.distance(current.0) / dt;
                    count += 1;
                }
            }
            last = Some(current);
        }
        if count == 0 { total } else { total / count as f32 }
    }
    /// `averagePosition`.
    pub fn average_position(&self, seconds: f64) -> Vec3 {
        let mut sum = Vec3::ZERO;
        let mut count = 0;
        for e in self.recent(seconds) {
            sum += e.0;
            count += 1;
        }
        if count == 0 { self.latest() } else { sum / count as f32 }
    }
}

/// Every tracker's state, and the room histories they share (`MCVR.hmdPivotHistory`,
/// `controllerHistory`).
#[derive(Default)]
pub(super) struct Trackers {
    pub hmd_pivot_history: History,
    pub controller_history: [History; 2],
    pub jump: JumpTracker,
    pub sneak: SneakTracker,
    pub climb: ClimbTracker,
    pub swing: SwingTracker,
    pub eating: EatingTracker,
    pub bow: BowTracker,
    pub interact: InteractTracker,
    pub backpack: BackpackTracker,
    pub swim: SwimTracker,
    pub teleport: super::vr_teleport::TeleportTracker,
    /// The triggers (main, offhand) at the last tick, for presses.
    pub last_trigger: [bool; 2],
    /// Inputs for the next tick's movement keys (`player.jumpFromGround`, `sneakOverride`).
    pub jump_request: bool,
}

#[derive(Default)]
pub(super) struct JumpTracker {
    c0_latched: bool,
    c1_latched: bool,
}

impl JumpTracker {
    pub fn is_jumping(&self) -> bool {
        self.c0_latched || self.c1_latched
    }
}

#[derive(Default)]
pub(super) struct SneakTracker {
    pub sneak_override: bool,
    pub sneak_counter: i32,
}

#[derive(Default)]
pub(super) struct ClimbTracker {
    pub latched: [bool; 2],
    was_latched: [bool; 2],
    was_in_block: [bool; 2],
    was_button: [bool; 2],
    in_block: [bool; 2],
    grab_direction: [Option<Grab>; 2],
    latch_start: [[f64; 3]; 2],
    latch_start_room: [Vec3; 2],
    latch_block: [[i32; 3]; 2],
    latch_start_controller: Option<usize>,
    /// `gravityOverride`: the player hangs, no gravity, while a hand holds on.
    pub hanging: bool,
}

/// The side of a ladder or vine a hand grabbed (`grabDirection`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Grab {
    North,
    South,
    East,
    West,
}

#[derive(Default)]
pub(super) struct BackpackTracker {
    was_in: [bool; 2],
    /// `KeybindHandler.PREVIOUS_ITEM_SLOT`: the slot the quick swap came from.
    previous_slot: usize,
}

#[derive(Default)]
pub(super) struct SwimTracker {
    motion: Vec3,
    last_dist: f64,
}

#[derive(Default)]
pub(super) struct EatingTracker {
    pub eating: [bool; 2],
    eat_start: Option<Instant>,
}

#[derive(Default)]
pub(super) struct BowTracker {
    pub active: bool,
    can_draw: bool,
    pub drawing: bool,
    was_drawing: bool,
    ts_notch: Option<Instant>,
    start_draw: Option<Instant>,
    current_draw: f32,
    max_draw: f32,
    /// `aim`: from the arrow hand to the bow hand, the way the arrow flies.
    pub aim: Option<Vec3>,
    last_hap_step: i32,
    hap_counter: i32,
}

impl BowTracker {
    fn draw_percent(&self) -> f32 {
        if self.max_draw > 0.0 { self.current_draw / self.max_draw } else { 0.0 }
    }
    pub fn is_notched(&self) -> bool {
        self.can_draw || self.drawing
    }
}

/// `InteractTracker` with its block and entity modules: a hand in something usable uses it.
#[derive(Default)]
pub(super) struct InteractTracker {
    /// What each hand (main, offhand) is in: a block it touches, or a mob.
    pub active: [Option<Touch>; 2],
}

#[derive(Clone, Copy)]
pub(super) enum Touch {
    Block(Hit),
    Mob(usize),
    /// `InteractiveHotbarModule`: a hotbar slot (0..9), or 9 the offhand slot.
    Hotbar(usize),
}

/// `InteractiveHotbarModule.previousHotbar`, for the haptic on a new slot.
static LAST_HOTBAR: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

impl ClimbTracker {
    pub fn is_grabbing_ladder(&self) -> bool {
        self.latched[0] || self.latched[1]
    }
}

/// `SwingTracker`: hands that hit blocks and mobs fast enough break and attack them.
pub(super) struct SwingTracker {
    pub disable_swing: i32,
    last_hand_rot: [Option<Quat>; 2],
    last_hand_pos: [Option<[f64; 3]>; 2],
    last_weapon_solid: [bool; 2],
    last_hit_entities: [Vec<u32>; 2],
    mining_point: [Option<[f64; 3]>; 2],
    attacking_point: [Option<[f64; 3]>; 2],
    weapon_tip: [Option<[f64; 3]>; 2],
    tip_history: [History; 2],
    /// `lastHitEffectPos`: the block a roomscale hit is breaking.
    last_hit: Option<[i32; 3]>,
}

impl Default for SwingTracker {
    fn default() -> Self {
        Self {
            disable_swing: 3,
            last_hand_rot: [None; 2],
            last_hand_pos: [None; 2],
            last_weapon_solid: [false; 2],
            last_hit_entities: [Vec::new(), Vec::new()],
            mining_point: [None; 2],
            attacking_point: [None; 2],
            weapon_tip: [None; 2],
            tip_history: Default::default(),
            last_hit: None,
        }
    }
}

/// `SwingTracker.SPEED_THRESH`: 2.5 m/s at 0.3 m along the hand is an intended smack.
const SPEED_THRESH: f32 = 2.5;

/// What a hand holds, as the swing tracker sorts it.
#[derive(Clone, Copy, PartialEq)]
enum Held {
    Empty,
    Item,
    Tool,
    Sword,
}

fn held_kind(item: Option<&str>) -> Held {
    let Some(item) = item else {
        return Held::Empty;
    };
    let name = item.trim_start_matches("minecraft:");
    if name.ends_with("_sword") || name == "trident" || name.ends_with("_spear") || name == "mace" {
        Held::Sword
    } else if name.ends_with("_pickaxe")
        || name.ends_with("_axe")
        || name.ends_with("_shovel")
        || name.ends_with("_hoe")
        || matches!(
            name,
            "shears"
                | "fishing_rod"
                | "carrot_on_a_stick"
                | "warped_fungus_on_a_stick"
                | "bone"
                | "blaze_rod"
                | "bamboo"
                | "torch"
                | "redstone_torch"
                | "stick"
                | "debug_stick"
                | "flint_and_steel"
                | "brush"
        )
        || name.ends_with("arrow")
    {
        Held::Tool
    } else {
        Held::Item
    }
}

fn face_index(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        _ => 5,
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn along(p: [f64; 3], d: Vec3, len: f32) -> [f64; 3] {
    [
        p[0] + f64::from(d.x * len),
        p[1] + f64::from(d.y * len),
        p[2] + f64::from(d.z * len),
    ]
}

fn wv(p: [f64; 3]) -> WVec3 {
    WVec3::new(p[0], p[1], p[2])
}

fn bounds(a: [f64; 3], b: [f64; 3]) -> Aabb {
    Aabb::new(
        WVec3::new(a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])),
        WVec3::new(a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])),
    )
}

fn include(b: Aabb, p: [f64; 3]) -> Aabb {
    let lo = b.min_position();
    let hi = b.max_position();
    Aabb::new(
        WVec3::new(lo.x.min(p[0]), lo.y.min(p[1]), lo.z.min(p[2])),
        WVec3::new(hi.x.max(p[0]), hi.y.max(p[1]), hi.z.max(p[2])),
    )
}

impl Game<'_> {
    /// The trackers' tick (`VRPlayer.tick`'s `PER_TICK` trackers).
    pub(super) fn vr_tick_trackers(&mut self, poses: Poses) -> horizon_client::Result<()> {
        let Some(data) = vr_player().as_ref().and_then(|v| v.pre) else {
            return Ok(());
        };
        let data = VrData {
            poses,
            room: data.room,
        };
        // The radial menu is a button, not roomscale: seated players have it too.
        self.vr_radial_tick();
        // Seated mode has no roomscale tracker input.  Clear the states that a previous
        // standing tick could have left behind before returning.
        if super::vr_settings::vr_settings().seated {
            super::vr_run::vr_run_reset();
            if let Some(player) = self.player.as_mut() {
                player.vr_crawl = false;
            }
            if self.serving.is_none() {
                super::vr_crawl::vr_crawl_stop();
            }
            if let Some(vr) = vr_player().as_mut() {
                vr.trackers.sneak.sneak_override = false;
            }
            horizon_vr::set_telescope(None);
            return Ok(());
        }
        self.vr_run_tracker();
        self.vr_crawl_tracker();
        self.vr_camera_tick(data);
        // `TelescopeTracker`: the hand holding a spyglass (the main hand first) shows its zoom.
        horizon_vr::set_telescope(
            (0..2).find(|&c| self.vr_hand_stack(c).is_some_and(|s| s.item.ends_with("spyglass"))),
        );
        {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            // `MCVR.updateAim`'s histories, in the room.
            let hmd = poses.hmd;
            let pivot = hmd.rotation() * Vec3::new(0.0, -0.1, 0.1) + hmd.translation();
            vr.trackers.hmd_pivot_history.add(pivot);
            for c in 0..2 {
                if let Some(aim) = poses.aim[c] {
                    vr.trackers.controller_history[c].add(aim.translation());
                }
            }
        }
        self.vr_jump_tracker();
        self.vr_sneak_tracker();
        self.vr_climb_tracker(data)?;
        self.vr_bow_tracker(data)?;
        self.vr_eating_tracker(data)?;
        self.vr_interact_tracker(data)?;
        self.vr_backpack_tracker(data);
        self.vr_swim_tracker(data);
        self.vr_teleport_tracker(data)?;
        self.vr_swing_tracker(data)?;
        if let (Some(vr), Some(pad)) = (vr_player().as_mut(), horizon_vr::pad()) {
            vr.trackers.last_trigger = [pad.triggers[1] > 0.5, pad.triggers[0] > 0.5];
        }
        Ok(())
    }

    fn vr_on_ground_alive(&self) -> bool {
        self.screen != Screen::Dead
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.ground.on_ground && !p.spectator)
            && self.riding.is_none()
    }

    /// `JumpTracker.activeProcess` without jump boots: the head rising fast above standing
    /// height jumps.
    fn vr_jump_tracker(&mut self) {
        if self.vr_boots_jump() {
            return;
        }
        let in_fluid = self
            .player
            .as_ref()
            .is_some_and(|p| p.fluids.was_touching_water);
        let shift = self.player.as_ref().is_some_and(|p| p.shift_key_down);
        let active = self.vr_on_ground_alive() && !in_fluid && !shift && super::vr_settings::vr_settings().jump;
        let mut guard = vr_player();
        let Some(vr) = guard.as_mut() else {
            return;
        };
        let t = &mut vr.trackers;
        if !active {
            t.jump.c0_latched = false;
            t.jump.c1_latched = false;
            return;
        }
        if t.hmd_pivot_history.net_movement(0.25).y > 0.1
            && t.hmd_pivot_history.latest().y - PLAYER_HEIGHT > JUMP_THRESHOLD
        {
            t.jump_request = true;
        }
    }

    /// `SneakTracker.activeProcess`: the head low enough below standing height sneaks.
    fn vr_sneak_tracker(&mut self) {
        let on_ladder = self.player.as_ref().is_some_and(|p| p.on_climbable);
        let active = self.screen != Screen::Dead
            && super::vr_settings::vr_settings().sneak
            && self.riding.is_none()
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.ground.on_ground || on_ladder);
        let mut guard = vr_player();
        let Some(vr) = guard.as_mut() else {
            return;
        };
        let t = &mut vr.trackers;
        if !active {
            t.sneak.sneak_override = false;
            return;
        }
        if t.sneak.sneak_counter > 0 {
            t.sneak.sneak_counter -= 1;
        }
        t.sneak.sneak_override =
            PLAYER_HEIGHT - t.hmd_pivot_history.latest().y > SNEAK_THRESHOLD;
    }

    /// Movement keys from the trackers for this tick: `(jump, sneak)`.
    pub(super) fn vr_movement_keys(&mut self) -> (bool, bool) {
        let mut guard = vr_player();
        let Some(vr) = guard.as_mut() else {
            return (false, false);
        };
        let jump = std::mem::take(&mut vr.trackers.jump_request);
        (jump, vr.trackers.sneak.sneak_override)
    }

    /// `SwingTracker.activeProcess`.
    fn vr_swing_tracker(&mut self, data: VrData) -> horizon_client::Result<()> {
        let active = {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            let swing = &mut vr.trackers.swing;
            let jumping = vr.trackers.jump.is_jumping();
            if swing.disable_swing > 0 {
                swing.disable_swing -= 1;
                false
            } else {
                // `WeaponCollision.AUTO`: off in creative.
                !jumping
                    && self.screen == Screen::Playing
                    && self.sleeping.is_none()
                    && !self.creative
                    && self.player.as_ref().is_some_and(|p| !p.spectator)
            }
        };
        if !active {
            let had_hit = {
                let mut guard = vr_player();
                let Some(vr) = guard.as_mut() else {
                    return Ok(());
                };
                let s = &mut vr.trackers.swing;
                s.last_hand_pos = [None; 2];
                s.attacking_point = [None; 2];
                s.weapon_tip = [None; 2];
                s.last_hit.take().is_some()
            };
            if had_hit {
                self.vr_stop_mining();
            }
            return Ok(());
        }
        // A roomscale hit the player walked away from (reach + 2 blocks) stops.
        if let (Some(pos), Some(p)) = (
            vr_player().as_ref().and_then(|v| v.trackers.swing.last_hit),
            self.player.as_ref().map(|p| p.position),
        ) {
            let c = [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + 0.5, f64::from(pos[2]) + 0.5];
            let d = (c[0] - p.x).powi(2) + (c[1] - p.y).powi(2) + (c[2] - p.z).powi(2);
            let reach = self.block_reach + 2.0;
            if d > reach * reach {
                if let Some(vr) = vr_player().as_mut() {
                    vr.trackers.swing.last_hit = None;
                }
                self.vr_stop_mining();
            }
        }
        let threshold = SPEED_THRESH * if self.creative { 1.5 } else { 1.0 };
        let shift = self.player.as_ref().is_some_and(|p| p.shift_key_down);
        for i in 0..2 {
            let c = i;
            let hand = if c == 1 { Hand::Off } else { Hand::Main };
            let climbing = vr_player()
                .as_ref()
                .is_some_and(|v| v.trackers.climb.latched[c]);
            if climbing {
                continue;
            }
            let hand_pos = data.controller_pos(c);
            let grip_rot = data.poses.grip[c]
                .or(data.poses.aim[c])
                .map_or(data.poses.hmd.rotation(), |p| p.rotation());
            let world_rot = data.room.turn() * grip_rot;
            let hand_dir = world_rot * Vec3::NEG_Z;
            let item = match hand {
                Hand::Main => self.held().map(|s| s.item.to_string()),
                Hand::Off => self.offhand.as_ref().map(|s| s.item.to_string()),
            };
            let kind = held_kind(item.as_deref());
            let player_reach = (self.entity_reach.min(6.0) - 0.5) as f32;
            let (mut weapon_length, entity_reach_add) = match kind {
                Held::Sword => (0.6, player_reach - 0.6),
                Held::Tool => (0.35, player_reach * 0.62 - 0.35),
                Held::Item => (0.1, player_reach * 0.16 - 0.1),
                Held::Empty => (0.0, 0.3),
            };
            weapon_length *= data.room.scale;
            let prev_mining_point = vr_player()
                .as_ref()
                .and_then(|v| v.trackers.swing.mining_point[i]);
            let mining_point = along(hand_pos, hand_dir, weapon_length);
            // The speed, in the room: 30 cm along the hand.
            let (speed, can_act, last_attack, last_tip, last_hand) = {
                let mut guard = vr_player();
                let Some(vr) = guard.as_mut() else {
                    return Ok(());
                };
                let s = &mut vr.trackers.swing;
                s.mining_point[i] = Some(mining_point);
                let base = data.poses.aim[c].map_or(Vec3::ZERO, |p| p.translation());
                let tip = base + grip_rot * Vec3::NEG_Z * 0.3;
                s.tip_history[i].add(tip);
                let speed = s.tip_history[i].average_speed(0.33);
                let can_act = speed > threshold && !s.last_weapon_solid[i];
                (
                    speed,
                    can_act,
                    s.attacking_point[i],
                    s.weapon_tip[i],
                    s.last_hand_pos[i].zip(s.last_hand_rot[i]),
                )
            };
            // Entities first; no hitting around corners.
            let mut entity_act = can_act;
            if entity_act {
                let head = data.hmd_pos();
                let to = sub(hand_pos, head);
                let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
                if len > 1e-6 {
                    let dir = to.map(|v| v / len);
                    if self.clip_ray(head, dir, len).is_some() {
                        entity_act = false;
                    }
                }
            }
            let attacking_point = self.vr_constrain(hand_pos, mining_point);
            let weapon_tip =
                self.vr_constrain(hand_pos, along(hand_pos, hand_dir, weapon_length + entity_reach_add));
            let mut tip_box = bounds(hand_pos, weapon_tip);
            if let Some(p) = last_tip {
                tip_box = include(tip_box, p);
            }
            let _ = last_attack;
            let vehicle = self.riding;
            let mobs: Vec<(usize, u32)> = self
                .mobs
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.removed && Some(m.id) != vehicle && m.bounds().intersects(tip_box))
                .map(|(k, m)| (k, m.id))
                .collect();
            let mut in_entity = false;
            let already: Vec<u32> = vr_player()
                .as_ref()
                .map(|v| v.trackers.swing.last_hit_entities[i].clone())
                .unwrap_or_default();
            for &(k, id) in &mobs {
                if already.contains(&id) || !entity_act {
                    continue;
                }
                self.vr_attack_mob(k, hand);
                horizon_vr::haptic(c, 1000);
                in_entity = true;
            }
            {
                let mut guard = vr_player();
                if let Some(vr) = guard.as_mut() {
                    let s = &mut vr.trackers.swing;
                    s.attacking_point[i] = Some(attacking_point);
                    s.weapon_tip[i] = Some(weapon_tip);
                    if in_entity {
                        s.last_weapon_solid[i] = true;
                    }
                    s.last_hit_entities[i] = if speed > threshold {
                        mobs.iter().map(|m| m.1).collect()
                    } else {
                        Vec::new()
                    };
                }
            }
            let can_act = can_act && !in_entity;
            // Blocks: trace from the last tick's point to this one's, through the swing's arc.
            let mut block_hit: Option<Hit> = None;
            if can_act {
                let mut points = Vec::new();
                match (last_hand, prev_mining_point) {
                    (Some((last_pos, last_rot)), Some(prev)) => {
                        let dot = world_rot.dot(last_rot).abs().min(1.0);
                        let angle = 2.0 * dot.acos();
                        let subdivisions = (angle / std::f32::consts::PI * 8.0).floor() as i32;
                        points.push(prev);
                        for s in 1..subdivisions {
                            let t = s as f32 / subdivisions as f32;
                            let r = last_rot.slerp(world_rot, t);
                            let lerp_hand: [f64; 3] = std::array::from_fn(|k| {
                                last_pos[k] + (hand_pos[k] - last_pos[k]) * f64::from(t)
                            });
                            points.push(along(lerp_hand, r * Vec3::NEG_Z, weapon_length));
                        }
                    }
                    _ => points.push(mining_point),
                }
                points.push(mining_point);
                for w in points.windows(2) {
                    let (start, mut end) = (w[0], w[1]);
                    let d = sub(end, start);
                    if d.iter().map(|v| v * v).sum::<f64>() < 1.0e-7 {
                        end = end.map(|v| v + 0.001);
                    }
                    let d = sub(end, start);
                    let len = d.iter().map(|v| v * v).sum::<f64>().sqrt();
                    let Some(hit) = self.clip_ray(start, d.map(|v| v / len), len) else {
                        continue;
                    };
                    let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                    let name = self.res.facts().get(state).name.to_string();
                    // `swordBlockCollision`: a sword breaks only what it breaks at once.
                    let mineable = self.destroy_progress(hit.pos) >= 1.0;
                    // Ladders and vines are for climbing (`realisticClimbEnabled`).
                    let protected = !shift
                        && (name.ends_with("ladder")
                            || name.ends_with("vine")
                            || name.ends_with("vines")
                            || name.ends_with("vines_plant"));
                    if (kind == Held::Sword && !mineable) || protected {
                        continue;
                    }
                    block_hit = Some(hit);
                    break;
                }
            }
            {
                let mut guard = vr_player();
                if let Some(vr) = guard.as_mut() {
                    let s = &mut vr.trackers.swing;
                    s.last_hand_pos[i] = Some(hand_pos);
                    s.last_hand_rot[i] = Some(world_rot);
                }
            }
            match block_hit {
                Some(hit) if !hit.inside && can_act => {
                    let mut total_hits = 3;
                    total_hits += (speed - threshold).min(4.0) as u32;
                    let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                    let name = self.res.facts().get(state).name.to_string();
                    if let Some(vr) = vr_player().as_mut() {
                        vr.trackers.swing.last_weapon_solid[i] = true;
                    }
                    if vr_is_openable(&name) {
                        // Roomscale door punching (`doorHitting`).
                        self.vr_use_on(hit, hand)?;
                    } else if name.ends_with("note_block") {
                        self.vr_roomscale_mine(hit.pos, face_index(hit.face), 1, hand)?;
                    } else {
                        let still = self.vr_roomscale_mine(hit.pos, face_index(hit.face), total_hits, hand)?;
                        if let Some(vr) = vr_player().as_mut() {
                            vr.trackers.swing.last_hit = still.then_some(hit.pos);
                        }
                    }
                    horizon_vr::haptic(c, 250 * total_hits);
                }
                _ => {
                    if let Some(vr) = vr_player().as_mut() {
                        vr.trackers.swing.last_weapon_solid[i] = false;
                    }
                }
            }
        }
        let _ = yaw_of;
        Ok(())
    }

    /// `SwingTracker.constrain`: `end`, or where a block cuts the line to it.
    fn vr_constrain(&self, start: [f64; 3], end: [f64; 3]) -> [f64; 3] {
        let d = sub(end, start);
        let len = d.iter().map(|v| v * v).sum::<f64>().sqrt();
        if len < 1e-9 {
            return end;
        }
        self.clip_ray(start, d.map(|v| v / len), len)
            .map_or(end, |h| h.point)
    }
}

/// `SwingTracker.isOpenable`, by kind: doors, trapdoors and fence gates open to a punch.
fn vr_is_openable(name: &str) -> bool {
    (name.ends_with("_door") || name.ends_with("_trapdoor") || name.ends_with("_fence_gate"))
        && !name.contains("iron_")
}

#[allow(dead_code)]
fn unused(_: WVec3) {
    let _ = wv;
}

/// `ClimbTracker`'s grab boxes, in the block (`northBB`...): the face a ladder or vine hangs on.
fn grab_box(dir: Grab) -> [f64; 6] {
    match dir {
        Grab::North => [0.1, 0.0, 0.9, 0.9, 1.0, 1.1],
        Grab::South => [0.1, 0.0, -0.1, 0.9, 1.0, 0.1],
        Grab::West => [0.9, 0.0, 0.1, 1.1, 1.0, 0.9],
        Grab::East => [-0.1, 0.0, 0.1, 0.1, 1.0, 0.9],
    }
}

fn block_of(p: [f64; 3]) -> [i32; 3] {
    p.map(|v| v.floor() as i32)
}

impl Game<'_> {
    pub(super) fn vr_block_name(&self, pos: [i32; 3]) -> String {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.res.facts().get(state).name.to_string()
    }
    fn vr_block_property(&self, pos: [i32; 3], key: &str) -> Option<String> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get(key).cloned())
    }
    /// `isClimbableBlock`: ladders, vines and the climbable tag.
    fn vr_climbable(&self, pos: [i32; 3]) -> bool {
        let name = self.vr_block_name(pos);
        name.ends_with("ladder")
            || name.ends_with(":vine")
            || name.ends_with("_vines")
            || name.ends_with("_vines_plant")
            || name.ends_with("scaffolding")
    }
    /// The grab boxes of the climbable block at `pos`, with their sides.
    fn vr_grab_boxes(&self, pos: [i32; 3]) -> Vec<Grab> {
        let name = self.vr_block_name(pos);
        if name.ends_with("ladder") {
            return match self.vr_block_property(pos, "facing").as_deref() {
                Some("east") => vec![Grab::East],
                Some("north") => vec![Grab::North],
                Some("south") => vec![Grab::South],
                Some("west") => vec![Grab::West],
                _ => Vec::new(),
            };
        }
        if name.ends_with(":vine") {
            let solid = |d: [i32; 3]| {
                let n = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                !self.vr_block_name(n).ends_with("air")
            };
            let mut out = Vec::new();
            let on = |k: &str| self.vr_block_property(pos, k).as_deref() == Some("true");
            if on("north") && solid([0, 0, -1]) {
                out.push(Grab::South);
            }
            if on("east") && solid([1, 0, 0]) {
                out.push(Grab::West);
            }
            if on("south") && solid([0, 0, 1]) {
                out.push(Grab::North);
            }
            if on("west") && solid([-1, 0, 0]) {
                out.push(Grab::East);
            }
            return out;
        }
        // `climbingAutoGrab`: other climbables are held with the grab key only.
        Vec::new()
    }

    /// Moves the player, the room with them (`setPos` with the room following).
    pub(super) fn vr_set_player_pos(&mut self, at: [f64; 3]) {
        let Some(p) = self.player.as_mut() else {
            return;
        };
        let old = [p.position.x, p.position.y, p.position.z];
        p.position.x = at[0];
        p.position.y = at[1];
        p.position.z = at[2];
        if let Some(vr) = vr_player().as_mut() {
            for i in 0..3 {
                vr.room_origin[i] += at[i] - old[i];
            }
        }
    }

    /// The player's box moved to `at`.
    fn vr_player_box_at(&self, at: [f64; 3]) -> Option<Aabb> {
        let p = self.player.as_ref()?;
        let half = f64::from(p.dimensions.width) / 2.0;
        let height = f64::from(p.dimensions.height);
        Some(Aabb::new(
            WVec3::new(at[0] - half, at[1], at[2] - half),
            WVec3::new(at[0] + half, at[1] + height, at[2] + half),
        ))
    }

    /// `findValidPlayerPos`: the new position, else with an axis left where it was.
    fn vr_find_valid_pos(&mut self, new: [f64; 3], feedback: bool) -> horizon_client::Result<[f64; 3]> {
        let Some(old) = self.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]) else {
            return Ok(new);
        };
        for i in 0..8 {
            let mut c = new;
            match i {
                1 => c[1] = old[1],
                2 => c[2] = old[2],
                3 => c[0] = old[0],
                4 => {
                    c[2] = old[2];
                    c[0] = old[0];
                }
                5 => {
                    c[0] = old[0];
                    c[1] = old[1];
                }
                6 => {
                    c[1] = old[1];
                    c[2] = old[2];
                }
                7 => c = old,
                _ => {}
            }
            let Some(bb) = self.vr_player_box_at(c) else {
                return Ok(old);
            };
            if self.vr_no_collision(bb)? {
                if feedback && i > 1 {
                    horizon_vr::haptic(0, 100);
                    horizon_vr::haptic(1, 100);
                }
                return Ok(c);
            }
        }
        Ok(old)
    }

    /// `ClimbTracker.activeProcess` (ladders and vines, `climbingAutoGrab`): a hand on the
    /// climbable face holds on, the player hangs from it and climbs as the hand moves, and lets
    /// go onto the top.
    fn vr_climb_tracker(&mut self, data: VrData) -> horizon_client::Result<()> {
        let still = self
            .player
            .as_ref()
            .is_some_and(|p| p.move_vector == [0.0, 0.0]);
        // Climb claws climb while walking too (`isActive`: claws or a still player).
        let still = still || super::vr_climb_claws::vr_claws_held(self);
        let active = self.screen != Screen::Dead
            && super::vr_settings::vr_settings().climb
            && self.riding.is_none()
            && still;
        if !active {
            if let Some(vr) = vr_player().as_mut() {
                let t = &mut vr.trackers.climb;
                t.latch_start_controller = None;
                t.latched = [false; 2];
                t.was_latched = [false; 2];
                t.hanging = false;
            }
            return Ok(());
        }
        let mut button = [false; 2];
        let mut allowed = [false; 2];
        let mut controller_pos = [[0.0; 3]; 2];
        let mut boxes_block = [[0; 3]; 2];
        let mut grab_dir = [None; 2];
        let pad = horizon_vr::pad();
        for c in 0..2 {
            // Climb claws: any solid block the hand is in, held with that hand's trigger
            // (`keyClimbeyGrab`).
            if self.vr_claws_in(c) {
                let pos = data.controller_pos(c);
                let block = block_of(pos);
                let name = self.vr_block_name(block);
                let solid = !(name.ends_with("air") || name.ends_with("water") || name.ends_with("lava"))
                    && (self.has_singleplayer_server() || super::vr_net::server_claws_block(&name));
                let trigger = pad.is_some_and(|p| p.triggers[1 - c] > 0.5);
                controller_pos[c] = pos;
                boxes_block[c] = block;
                button[c] = solid && trigger;
                allowed[c] = solid;
                if let Some(vr) = vr_player().as_mut() {
                    vr.trackers.climb.in_block[c] = solid;
                }
                continue;
            }
            controller_pos[c] = data.controller_pos(c);
            let dir = data.controller_dir(c);
            let near = along(controller_pos[c], dir, -0.2);
            let mut block = block_of(controller_pos[c]);
            let mut ok = self.vr_climbable(block);
            if !ok {
                let b2 = block_of(near);
                if self.vr_climbable(b2) {
                    ok = true;
                    block = b2;
                    controller_pos[c] = near;
                }
            }
            let was_ok = ok;
            let mut in_block = false;
            if ok {
                let hand_box = bounds(controller_pos[c], near);
                for g in self.vr_grab_boxes(block) {
                    let b = grab_box(g);
                    let o = block.map(f64::from);
                    let grab = Aabb::new(
                        WVec3::new(o[0] + b[0], o[1] + b[1], o[2] + b[2]),
                        WVec3::new(o[0] + b[3], o[1] + b[4], o[2] + b[5]),
                    );
                    if hand_box.intersects(grab) {
                        in_block = true;
                        grab_dir[c] = Some(g);
                        break;
                    }
                }
            }
            if !was_ok {
                let guard = vr_player();
                let t = guard.as_ref().map(|v| (v.trackers.climb.latch_start[c], v.trackers.climb.was_in_block[c]));
                drop(guard);
                if let Some((latch, was_in)) = t {
                    let d = sub(latch, controller_pos[c]);
                    if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > 0.5 {
                        in_block = false;
                    } else {
                        in_block = was_in && self.vr_climbable(block_of(latch));
                    }
                }
            }
            boxes_block[c] = block;
            button[c] = in_block;
            allowed[c] = in_block;
            if let Some(vr) = vr_player().as_mut() {
                vr.trackers.climb.in_block[c] = in_block;
            }
        }
        let player_pos = self.player.as_ref().map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let mut grabbed = false;
        {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            let t = &mut vr.trackers.climb;
            let mut nope = false;
            for c in 0..2 {
                t.was_latched[c] = t.latched[c];
                if !button[c] && t.latched[c] {
                    t.latched[c] = false;
                }
                if !t.latched[c] && !nope && allowed[c] {
                    if !t.was_in_block[c] && t.in_block[c] {
                        horizon_vr::haptic(c, 750);
                    }
                    if (!t.was_in_block[c] && t.in_block[c] && button[c])
                        || (!t.was_button[c] && button[c] && t.in_block[c])
                    {
                        grabbed = true;
                        t.latch_start[c] = controller_pos[c];
                        t.latch_start_room[c] = data.poses.aim[c].map_or(Vec3::ZERO, |p| p.translation());
                        t.latch_block[c] = boxes_block[c];
                        t.grab_direction[c] = grab_dir[c];
                        t.latch_start_controller = Some(c);
                        t.latched[c] = true;
                        if c == 0 {
                            t.latched[1] = false;
                            nope = true;
                        } else {
                            t.latched[0] = false;
                        }
                        horizon_vr::haptic(c, 2000);
                    }
                }
                t.was_button[c] = button[c];
                t.was_in_block[c] = t.in_block[c];
            }
            if !t.latched[0] && !t.latched[1] {
                for c in 0..2 {
                    if t.in_block[c] && button[c] && allowed[c] {
                        grabbed = true;
                        t.latch_start[c] = controller_pos[c];
                        t.latch_start_room[c] = data.poses.aim[c].map_or(Vec3::ZERO, |p| p.translation());
                        t.latch_block[c] = boxes_block[c];
                        t.grab_direction[c] = grab_dir[c];
                        t.latch_start_controller = Some(c);
                        t.latched[c] = true;
                        horizon_vr::haptic(c, 2000);
                    }
                }
            }
            t.hanging = t.latched[0] || t.latched[1];
            if !t.hanging {
                t.latch_start_controller = None;
                return Ok(());
            }
        }
        let _ = player_pos;
        // Climb by the hand's move since last tick.
        let (c, start_room, latch_start, latch_block, grab) = {
            let guard = vr_player();
            let Some(vr) = guard.as_ref() else {
                return Ok(());
            };
            let t = &vr.trackers.climb;
            let Some(c) = t.latch_start_controller else {
                return Ok(());
            };
            (c, t.latch_start_room[c], t.latch_start[c], t.latch_block[c], t.grab_direction[c])
        };
        let now = data.controller_pos(c);
        let start = data.room.to_world(start_room);
        let delta = sub(now, start);
        if let Some(vr) = vr_player().as_mut() {
            vr.trackers.climb.latch_start_room[c] = data.poses.aim[c].map_or(Vec3::ZERO, |p| p.translation());
        }
        let Some(p) = self.player.as_mut() else {
            return Ok(());
        };
        if grabbed {
            p.velocity = WVec3::new(0.0, 0.0, 0.0);
        } else {
            p.velocity = WVec3::new(p.velocity.x, 0.0, p.velocity.z);
        }
        p.fall_distance = 0.0;
        let (x, y, z) = (p.position.x, p.position.y, p.position.z);
        let mut new = [x, y - delta[1], z];
        let scale = f64::from(data.room.scale.min(1.0));
        match grab {
            Some(Grab::North) | Some(Grab::South) => {
                new[0] = x - delta[0];
                new[2] = f64::from(latch_block[2]) + 0.5
                    + (1.0 - scale) * if grab == Some(Grab::North) { 0.5 } else { -0.5 };
            }
            Some(Grab::East) | Some(Grab::West) => {
                new[2] = z - delta[2];
                new[0] = f64::from(latch_block[0]) + 0.5
                    + (1.0 - scale) * if grab == Some(Grab::West) { 0.5 } else { -0.5 };
            }
            None => {
                new[0] = x - delta[0];
                new[2] = z - delta[2];
            }
        }
        // Hands low, holding the top fifth: step up onto the top.
        let hmd_pivot_room = (data.poses.hmd.rotation() * Vec3::new(0.0, -0.1, 0.1) + data.poses.hmd.translation()).y;
        let controller_room = data.poses.aim[c].map_or(0.0, |p| p.position[1]);
        let top = f64::from(latch_block[1]) + 1.0;
        if controller_room <= hmd_pivot_room / 2.0 && latch_start[1] > 0.8 + f64::from(latch_block[1]) {
            let mut h = data.hmd_dir();
            h.y = 0.0;
            let h = h.normalize_or_zero() * 0.1;
            let up = [x + f64::from(h.x), top, z + f64::from(h.z)];
            if let Some(bb) = self.vr_player_box_at(up)
                && self.vr_no_collision(bb)?
            {
                new = up;
                if let Some(vr) = vr_player().as_mut() {
                    let t = &mut vr.trackers.climb;
                    t.latch_start_controller = None;
                    t.latched = [false; 2];
                    t.was_in_block = [false; 2];
                    t.hanging = false;
                }
            }
        }
        let valid = self.vr_find_valid_pos(new, true)?;
        self.vr_set_player_pos(valid);
        Ok(())
    }
}

/// Whether the hands' item is in use by a tracker (`ItemInUseTracker.itemInUse`): eating or
/// drawing keeps the use going without the use button.
pub(super) fn vr_item_in_use() -> bool {
    vr_player()
        .as_ref()
        .is_some_and(|v| v.trackers.eating.eating.iter().any(|e| *e) || v.trackers.bow.drawing)
}

/// The triggers (left, right as a pad has them) a tracker took this tick: the bow's draw and a
/// hand's touch use them instead of attacking and using.
pub(super) fn vr_taken_triggers() -> [bool; 2] {
    vr_player().as_ref().map_or([false; 2], |v| {
        let t = &v.trackers;
        let main = t.bow.active && t.bow.is_notched()
            || t.interact.active[0].is_some()
            || super::vr_camera::vr_camera_touched()
            || super::vr_radial::vr_radial_showing();
        let off = t.interact.active[1].is_some();
        [off, main]
    })
}

impl Game<'_> {
    fn vr_hand_stack(&self, c: usize) -> Option<horizon_client::items::ItemStack> {
        if c == 0 {
            self.held().cloned()
        } else {
            self.offhand.clone()
        }
    }
    fn vr_use_animation(&self, c: usize) -> horizon_core::use_animation::UseAnimation {
        self.vr_hand_stack(c)
            .map_or_else(Default::default, |s| self.item_use_animation(&s))
    }

    /// `EatingTracker.activeProcess`: food or a drink at the mouth is eaten.
    fn vr_eating_tracker(&mut self, data: VrData) -> horizon_client::Result<()> {
        use horizon_core::use_animation::UseAnimation;
        let bow = vr_player().as_ref().is_some_and(|v| v.trackers.bow.active);
        let edible = |a: UseAnimation| matches!(a, UseAnimation::Eat | UseAnimation::Drink | UseAnimation::TootHorn);
        let anims = [self.vr_use_animation(0), self.vr_use_animation(1)];
        let active = self.screen == Screen::Playing
            && self.sleeping.is_none()
            && !bow
            && (edible(anims[0]) || edible(anims[1]));
        if !active {
            if let Some(vr) = vr_player().as_mut() {
                vr.trackers.eating.eating = [false; 2];
            }
            return Ok(());
        }
        let hmd = data.poses.hmd;
        let mouth = hmd.translation();
        for c in 0..2 {
            if !edible(anims[c]) {
                continue;
            }
            let food = {
                let guard = vr_player();
                let Some(vr) = guard.as_ref() else {
                    return Ok(());
                };
                let dir = data.poses.aim[c].map_or(Vec3::NEG_Z, |p| p.direction());
                vr.trackers.controller_history[c].average_position(0.333) + dir * 0.2
            };
            let near = mouth.distance(food) < 0.25;
            if !near {
                if let Some(vr) = vr_player().as_mut() {
                    vr.trackers.eating.eating[c] = false;
                }
                continue;
            }
            // That's how liquid works: a drink is tipped.
            if anims[c] == UseAnimation::Drink {
                let up = data.poses.aim[c].map_or(Vec3::Y, |p| p.rotation() * Vec3::Y);
                if up.y > 0.0 {
                    if let Some(vr) = vr_player().as_mut() {
                        vr.trackers.eating.eating[c] = false;
                    }
                    continue;
                }
            }
            let crunch = match anims[c] {
                UseAnimation::Eat => 2,
                UseAnimation::TootHorn => 1,
                _ => 0,
            };
            let eating = vr_player().as_ref().is_some_and(|v| v.trackers.eating.eating[c]);
            if !eating {
                let was = self.using.is_some();
                self.use_with_hand()?;
                if !was && self.using.is_some() {
                    if let Some(vr) = vr_player().as_mut() {
                        vr.trackers.eating.eating[c] = true;
                        vr.trackers.eating.eat_start = Some(Instant::now());
                    }
                }
            }
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            let e = &mut vr.trackers.eating;
            if e.eating[c] {
                if let Some((_, ticks)) = self.using
                    && ticks % 5 <= crunch
                {
                    horizon_vr::haptic(c, 700);
                }
            }
            if e.eat_start.is_some_and(|t| t.elapsed().as_millis() > 2100) {
                e.eating[c] = false;
            }
        }
        Ok(())
    }

    /// `BowTracker.activeProcess` with `BowModule`: the arrow hand nocks at the bow's string,
    /// the trigger draws, pulling back draws further, letting go shoots along the arrow.
    fn vr_bow_tracker(&mut self, data: VrData) -> horizon_client::Result<()> {
        use horizon_core::use_animation::UseAnimation;
        let bow_main = self.vr_use_animation(0) == UseAnimation::Bow;
        let bow_off = self.vr_use_animation(1) == UseAnimation::Bow;
        let active = self.screen == Screen::Playing && self.sleeping.is_none() && (bow_main || bow_off);
        if !active {
            let release = vr_player().as_mut().is_some_and(|v| {
                let b = &mut v.trackers.bow;
                let r = b.drawing;
                b.active = false;
                b.can_draw = false;
                b.drawing = false;
                b.was_drawing = false;
                r
            });
            let _ = release;
            self.vr_net_bow(0.0, 0);
            return Ok(());
        }
        let has_ammo = self.inventory.iter().flatten().any(|s| s.item.ends_with("arrow"))
            || self.offhand.as_ref().is_some_and(|s| s.item.ends_with("arrow"))
            || self.creative;
        let height = self.player.as_ref().map_or(1.8, |p| p.dimensions.height);
        let max_draw = height * 0.22;
        let (bow_hand, arrow_hand) = (1usize, 0usize);
        let arrow_pos = data.controller_pos(arrow_hand);
        let bow_pos = data.controller_pos(bow_hand);
        let dist = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() as f32;
        let controllers_dist = dist(bow_pos, arrow_pos);
        let bow_rot = data.room.turn()
            * data.poses.grip[bow_hand].or(data.poses.aim[bow_hand]).map_or(Quat::IDENTITY, |p| p.rotation());
        let up = bow_rot * Vec3::new(0.0, data.room.scale, 0.0);
        let string_pos = along(bow_pos, up, max_draw * 0.5);
        let notch_dist = dist(arrow_pos, string_pos);
        let aim = {
            let d = sub(bow_pos, arrow_pos);
            Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32).normalize_or_zero()
        };
        let arrow_aim = data.controller_vector(arrow_hand, Vec3::NEG_Z);
        let bow_aim = bow_rot * Vec3::NEG_Y;
        let dot = bow_aim.dot(arrow_aim);
        let angle = if dot > 0.999 { 0.0 } else { dot.clamp(-1.0, 1.0).acos().to_degrees() };
        let notch_threshold = 0.15 * data.room.scale;
        let trigger = horizon_vr::pad().is_some_and(|p| p.triggers[1] > 0.5);
        let slot = if bow_main { self.selected } else { super::shield::OFFHAND_SLOT };
        let (start, release, draw_percent) = {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            let b = &mut vr.trackers.bow;
            b.active = true;
            b.max_draw = max_draw;
            b.aim = Some(aim);
            let last_can_draw = b.can_draw;
            if has_ammo && notch_dist <= notch_threshold && angle <= 20.0 {
                b.can_draw = true;
                b.ts_notch = Some(Instant::now());
            } else if !b.drawing && b.ts_notch.is_none_or(|t| t.elapsed().as_millis() > 500) {
                b.can_draw = false;
            }
            // The trigger on a nocked arrow draws; letting go shoots (`BowModule`).
            let start = !b.drawing && b.can_draw && trigger;
            let release = b.drawing && !trigger;
            if start {
                b.drawing = true;
                b.start_draw = Some(Instant::now());
            }
            if release {
                b.drawing = false;
            }
            if !b.drawing && b.can_draw && !last_can_draw {
                horizon_vr::haptic(arrow_hand, 800);
                horizon_vr::haptic(bow_hand, 800);
            }
            if b.drawing {
                b.current_draw = ((controllers_dist - notch_threshold) / data.room.scale).min(b.max_draw);
                let p = b.draw_percent();
                let strength = if p > 0.0 { (p * 500.0) as u32 + 700 } else { 0 };
                let step = (p * 4.0 * 4.0 * 3.0) as i32;
                if step % 2 == 0 && b.last_hap_step != step {
                    horizon_vr::haptic(arrow_hand, strength);
                    if p >= 1.0 {
                        horizon_vr::haptic(bow_hand, strength);
                    }
                }
                if b.start_draw.is_some_and(|t| t.elapsed().as_millis() >= 1100) && b.hap_counter % 4 == 0 {
                    horizon_vr::haptic(bow_hand, 200);
                }
                b.last_hap_step = step;
                b.hap_counter += 1;
            } else {
                b.hap_counter = 0;
                b.last_hap_step = 0;
            }
            b.was_drawing = b.drawing;
            (start, release, b.draw_percent())
        };
        let drawing = vr_player().as_ref().is_some_and(|v| v.trackers.bow.drawing);
        self.vr_net_bow(if drawing { draw_percent.max(0.01) } else { 0.0 }, arrow_hand);
        if start {
            self.use_with_hand()?;
        }
        if let Some((s, _)) = self.using
            && s == slot
            && !release
        {
            // How far the string is drawn is how long the bow was held (`setItemInUseRemainingClient`).
            let ticks = if draw_percent >= 1.0 { 25 } else if draw_percent > 0.4 { 15 } else { 0 };
            self.using = Some((slot, ticks));
        }
        if release {
            // The arrow flies along the bow's aim.
            let (yaw, pitch) = (yaw_of(aim), -super::vr_player::pitch_of(aim));
            let turn = super::vr_player::wrap_degrees(yaw - self.yaw);
            self.turn_player(f64::from(turn / 0.15), f64::from((pitch - self.pitch) / 0.15));
            horizon_vr::haptic(arrow_hand, 500);
            horizon_vr::haptic(bow_hand, 3000);
        }
        Ok(())
    }

    /// `InteractTracker.activeProcess` with the block and entity modules, and `processBindings`:
    /// a hand in a usable block or a mob uses it with the trigger.
    fn vr_interact_tracker(&mut self, data: VrData) -> horizon_client::Result<()> {
        let active = self.screen == Screen::Playing && self.sleeping.is_none();
        let mut touches = [None, None];
        if active {
            let pivot = data.head_pivot();
            for c in 0..2 {
                // The hotbar module first (main hand only).
                if c == 0
                    && let Some(slot) = self.vr_hotbar_touch(&data)
                {
                    touches[0] = Some(Touch::Hotbar(slot));
                    continue;
                }
                let hand = data.controller_pos(c);
                // The entity module first: a hand in a mob.
                let point = WVec3::new(hand[0], hand[1], hand[2]);
                let vehicle = self.riding;
                if let Some((k, _)) = self
                    .mobs
                    .iter()
                    .enumerate()
                    .find(|(_, m)| !m.removed && Some(m.id) != vehicle && m.bounds().contains(point))
                {
                    touches[c] = Some(Touch::Mob(k));
                    continue;
                }
                // The block module: the head's line to the hand meets a usable block's shape.
                let block = block_of(hand);
                let name = self.vr_block_name(block);
                if !vr_right_clickable(&name) {
                    continue;
                }
                let d = sub(hand, pivot);
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len < 1e-6 {
                    continue;
                }
                if let Some(hit) = self.clip_ray(pivot, d.map(|v| v / len), len + 0.05)
                    && hit.pos == block
                {
                    touches[c] = Some(Touch::Block(hit));
                }
            }
        }
        let (pressed, was_active) = {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return Ok(());
            };
            let pad = horizon_vr::pad();
            let now = pad.map_or([false; 2], |p| [p.triggers[1] > 0.5, p.triggers[0] > 0.5]);
            let last = vr.trackers.last_trigger;
            let was = vr.trackers.interact.active.map(|t| t.is_some());
            vr.trackers.interact.active = touches;
            ([now[0] && !last[0], now[1] && !last[1]], was)
        };
        let hotbar = match touches[0] {
            Some(Touch::Hotbar(i)) => i as i8,
            _ => -1,
        };
        if LAST_HOTBAR.swap(hotbar, std::sync::atomic::Ordering::Relaxed) != hotbar && hotbar >= 0 {
            horizon_vr::haptic(0, 750);
        }
        for c in 0..2 {
            if !was_active[c] && touches[c].is_some_and(|t| !matches!(t, Touch::Hotbar(_))) {
                horizon_vr::haptic(c, 250);
            }
            if !pressed[c] {
                continue;
            }
            let hand = if c == 0 { Hand::Main } else { Hand::Off };
            match touches[c] {
                Some(Touch::Block(hit)) => {
                    self.vr_use_on(hit, hand)?;
                    self.swing_hand(hand, horizon_client::first_person::SwingAnimation::DEFAULT);
                    horizon_vr::haptic(c, 750);
                }
                Some(Touch::Mob(k)) => {
                    self.vr_interact_mob(k)?;
                    horizon_vr::haptic(c, 750);
                }
                // `onPress`: select the slot, or swap with the offhand.
                Some(Touch::Hotbar(9)) => drop(self.click_mapping("key.swapOffhand")),
                Some(Touch::Hotbar(i)) if self.selected != i => self.hotbar_presses.push(i),
                Some(Touch::Hotbar(_)) | None => {}
            }
        }
        Ok(())
    }

    /// `InteractiveHotbarModule.isActive`: the main hand within 6 cm of the wrist HUD's hotbar,
    /// and the slot it is over.
    // shortcut: spectators' command bar isn't touched (it has no touch slots here).
    fn vr_hotbar_touch(&self, data: &VrData) -> Option<usize> {
        let settings = super::vr_settings::vr_settings();
        if settings.seated || self.spectator() {
            return None;
        }
        if self.vr_claws_in(0)
            && vr_player().as_ref().is_some_and(|v| v.trackers.climb.is_grabbing_ladder())
        {
            return None;
        }
        let (centre, rot, size) = horizon_vr::wrist_hud()?;
        let main = data.poses.aim[0]?.translation();
        // The HUD picture is the whole GUI: the hotbar is 182 px wide, centred, 11 px up.
        let (w, h) = self.gui_size();
        let local = |px: f32, py: f32| {
            centre + rot * glam::Vec3::new((px / w - 0.5) * size[0], (0.5 - py / h) * size[1], 0.0)
        };
        let start = local(w / 2.0 - 91.0, h - 11.0);
        let end = local(w / 2.0 + 91.0, h - 11.0);
        let line = end - start;
        let to_hand = main - start;
        if to_hand.cross(line).length() / line.length() > 0.06 {
            return None;
        }
        let pos = to_hand.dot(line) / line.length_squared() * 9.0;
        let reversed = horizon_vr::reversed();
        match pos.floor() as i32 {
            b @ 0..=8 => Some(b as usize),
            // The offhand slot, left of the bar (right when the hands are reversed).
            _ if !reversed && (-1.5..=-0.5).contains(&pos) => Some(9),
            _ if reversed && (9.5..=10.5).contains(&pos) => Some(9),
            _ => None,
        }
    }
}

/// `BlockInteractionModule.rightClickable`: blocks with their own use (`useWithoutItem`).
fn vr_right_clickable(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    n.ends_with("_door")
        || n.ends_with("_trapdoor")
        || n.ends_with("_fence_gate")
        || n.ends_with("_button")
        || n.ends_with("_bed")
        || n.ends_with("_sign")
        || n.ends_with("shulker_box")
        || n.ends_with("_anvil")
        || n.ends_with("anvil")
        || n.ends_with("chest")
        || n.ends_with("candle")
        || n.ends_with("_cake")
        || matches!(
            n,
            "lever"
                | "barrel"
                | "crafting_table"
                | "furnace"
                | "blast_furnace"
                | "smoker"
                | "loom"
                | "cartography_table"
                | "smithing_table"
                | "stonecutter"
                | "grindstone"
                | "enchanting_table"
                | "brewing_stand"
                | "beacon"
                | "hopper"
                | "dispenser"
                | "dropper"
                | "repeater"
                | "comparator"
                | "daylight_detector"
                | "note_block"
                | "jukebox"
                | "bell"
                | "cake"
                | "composter"
                | "lectern"
                | "respawn_anchor"
                | "crafter"
                | "decorated_pot"
                | "chiseled_bookshelf"
                | "flower_pot"
                | "cauldron"
                | "water_cauldron"
                | "lava_cauldron"
                | "powder_snow_cauldron"
                | "dragon_egg"
                | "command_block"
                | "chain_command_block"
                | "repeating_command_block"
                | "structure_block"
                | "jigsaw"
        )
}

impl Game<'_> {
    /// `BackpackTracker.activeProcess`: a hand reaching down behind the head, pointing down,
    /// quick-swaps the main hand to the first slot and back, or swaps the offhand.
    fn vr_backpack_tracker(&mut self, data: VrData) {
        let drawing = vr_player().as_ref().is_some_and(|v| v.trackers.bow.drawing);
        let trigger = horizon_vr::pad().is_some_and(|p| p.triggers[1] > 0.5);
        if self.screen != Screen::Playing || self.sleeping.is_some() || drawing || trigger {
            return;
        }
        let p = data.poses;
        // In the room (`vrdata_room_pre`): the head's rear, the hands and the head's look.
        let hmd_rot = p.hmd.rotation();
        let rear = p.hmd.translation() + hmd_rot * Vec3::new(0.0, -0.2, 0.2);
        let hmd_dir = p.hmd.direction();
        let mut presses = [false; 2];
        {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                return;
            };
            let b = &mut vr.trackers.backpack;
            for c in 0..2 {
                let Some(aim) = p.aim[c] else {
                    continue;
                };
                let hand_dir = p.grip[c].unwrap_or(aim).direction();
                let delta = rear - aim.translation();
                let dot = hand_dir.dot(Vec3::NEG_Y);
                let dot_delta = delta.dot(hmd_dir);
                let below = (rear.y - aim.position[1]).abs() < 0.25;
                let behind = dot_delta > 0.0 && delta.length() > 0.05;
                let aim_down = dot > 0.6;
                let in_front = dot_delta < 0.0 && delta.length() > 0.25;
                let aim_up = dot < 0.0;
                if below && behind && aim_down {
                    if !b.was_in[c] {
                        presses[c] = true;
                        horizon_vr::haptic(c, 1500);
                        b.was_in[c] = true;
                    }
                } else if in_front || aim_up {
                    b.was_in[c] = false;
                }
            }
        }
        if presses[0] {
            // `keyQuickSwap`: to the first slot, and back to where it came from.
            let previous = vr_player().as_ref().map_or(0, |v| v.trackers.backpack.previous_slot);
            let to = if self.selected != 0 {
                if let Some(vr) = vr_player().as_mut() {
                    vr.trackers.backpack.previous_slot = self.selected;
                }
                0
            } else {
                if let Some(vr) = vr_player().as_mut() {
                    vr.trackers.backpack.previous_slot = 0;
                }
                previous
            };
            self.hotbar_presses.push(to);
        }
        if presses[1] {
            self.vr_swap_offhand();
        }
    }

    /// `SwimTracker.activeProcess`: breaststrokes, the hands moving apart in front of the head,
    /// push the player through water along the strokes.
    fn vr_swim_tracker(&mut self, data: VrData) {
        let (in_water, still) = self.player.as_ref().map_or((false, false), |p| {
            (p.fluids.was_touching_water, p.move_vector == [0.0, 0.0])
        });
        if self.screen != Screen::Playing || !in_water || !still || !super::vr_settings::vr_settings().swim {
            return;
        }
        let r = data.controller_pos(0);
        let l = data.controller_pos(1);
        let middle: [f64; 3] = std::array::from_fn(|i| (l[i] - r[i]) * 0.5 + r[i]);
        let pivot = data.head_pivot();
        let hmd = [pivot[0], pivot[1] - 0.3, pivot[2]];
        let to = sub(middle, hmd);
        let move_dir = (Vec3::new(to[0] as f32, to[1] as f32, to[2] as f32).normalize_or_zero() + data.hmd_dir()) * 0.5;
        let controller_dir = (data.controller_vector(0, Vec3::NEG_Z) + data.controller_vector(1, Vec3::NEG_Z)) * 0.5;
        let dir_factor = (controller_dir + move_dir).length() * 0.5;
        let distance = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        let mut guard = vr_player();
        let Some(vr) = guard.as_mut() else {
            return;
        };
        let s = &mut vr.trackers.swim;
        let delta = s.last_dist - distance;
        if delta > 0.0 {
            s.motion += move_dir * (delta as f32 * 1.3 * dir_factor) * 0.15;
        }
        s.last_dist = distance;
        let m = s.motion;
        s.motion *= 0.9;
        drop(guard);
        if let Some(p) = self.player.as_mut() {
            p.velocity = WVec3::new(
                p.velocity.x + f64::from(m.x),
                p.velocity.y + f64::from(m.y),
                p.velocity.z + f64::from(m.z),
            );
        }
    }
}
