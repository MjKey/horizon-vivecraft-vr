//! VR Throwing Extensions (`demistorm/VR-Throwing-Extensions` 26.3): throw held items or stacks
//! using controller swing velocity on grip release (or boosted Drop key), ballistic aim assist,
//! thrown weapon damage + block embedding, boomerang tool/weapon return, and physical catching.
use super::vr_player::{VrData, vr_player};
use super::vr_settings::vr_settings;
use super::{Game, Screen};
use glam::Vec3;
use horizon_client::first_person::{Hand, SwingAnimation};
use horizon_core::geometry::{Aabb, Vec3 as WVec3};
use horizon_core::random::RandomSource as _;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const HOLD_ARM_MS: u128 = 120;
const MIN_THROW_SPEED_MPS: f32 = 1.6;
const MAX_THROW_SPEED_BPT: f32 = 1.8;
const AIM_MAX_DISTANCE: f64 = 24.0;
const AIM_MAX_COS: f32 = 0.951; // cos(18 deg)
const ITEM_GRAVITY_BPT2: f32 = 0.04;
const CATCH_RADIUS: f64 = 0.45;
const BOOMERANG_RETURN_TICKS: u32 = 18;
const BOOMERANG_RETURN_SPEED: f64 = 0.55;

/// Tracks which controller grips (0 main, 1 offhand) are currently armed for a throw gesture so
/// hotbar cycling does not trigger on release while throwing.
static GRIP_ARMED: Mutex<[bool; 2]> = Mutex::new([false; 2]);

pub(crate) fn vr_throw_takes_grip(c: usize) -> bool {
    c < 2 && GRIP_ARMED.lock().unwrap_or_else(|e| e.into_inner())[c]
}

pub(crate) fn vr_throw_reset() {
    *GRIP_ARMED.lock().unwrap_or_else(|e| e.into_inner()) = [false; 2];
}

/// Active thrown item metadata for weapon impact damage, block embedding, and boomerang return.
#[derive(Clone, Copy, Debug)]
pub(super) struct ThrownMeta {
    pub hand: usize,
    pub damage: f32,
    pub sharp: bool,
    pub boomerang: bool,
    pub returning: bool,
    pub ticks: u32,
    pub embedded: bool,
}

#[derive(Default)]
pub(super) struct ThrowTracker {
    hold_start: [Option<Instant>; 2],
    was_grip: [bool; 2],
    armed: [bool; 2],
    pub thrown: HashMap<u32, ThrownMeta>,
}

/// Converts room-rate hand velocity (m/s) + player velocity (blocks/tick) to Horizon item motion
/// (blocks/tick) scaled by `mult`, clamped to `MAX_THROW_SPEED_BPT` before adding player motion.
pub(crate) fn compute_throw_velocity(hand_vel: Vec3, player_vel: Vec3, mult: f32) -> Vec3 {
    let scaled = hand_vel * 0.05 * mult;
    let speed = scaled.length();
    let clamped = if speed > MAX_THROW_SPEED_BPT && speed > 1e-6 {
        scaled * (MAX_THROW_SPEED_BPT / speed)
    } else {
        scaled
    };
    clamped + player_vel
}

/// Returns `(base_damage, sharp_embeds_in_blocks, boomerang_eligible)` for a thrown item id.
pub(crate) fn weapon_throw_stats(item: &str, loyalty: u32) -> (f32, bool, bool) {
    let n = item.strip_prefix("minecraft:").unwrap_or(item);
    if n == "trident" {
        return (8.0, true, true);
    }
    if n.ends_with("_sword") {
        let dmg = match n {
            "netherite_sword" => 8.0,
            "diamond_sword" => 7.0,
            "iron_sword" => 6.0,
            "stone_sword" => 5.0,
            _ => 4.0,
        };
        return (dmg, true, true);
    }
    if n.ends_with("_axe") {
        let dmg = match n {
            "netherite_axe" => 10.0,
            "diamond_axe" | "iron_axe" | "stone_axe" => 9.0,
            _ => 7.0,
        };
        return (dmg, true, true);
    }
    if n.ends_with("_pickaxe") || n.ends_with("_shovel") || n.ends_with("_hoe") {
        return (4.5, true, true);
    }
    if n == "mace" {
        return (7.0, true, true);
    }
    (2.0, false, loyalty > 0)
}

/// `AimHelper` magnetic cone assist: steers `vel` within `18°` and `24` blocks toward the
/// closest target center with mild gravity arc compensation, preserving throw speed.
pub(crate) fn apply_aim_assist(origin: [f64; 3], vel: Vec3, targets: &[[f64; 3]]) -> Vec3 {
    let speed = vel.length();
    if speed <= 1e-4 {
        return vel;
    }
    let dir = vel / speed;
    let mut best: Option<(f64, Vec3)> = None;
    for &t in targets {
        let delta = Vec3::new(
            (t[0] - origin[0]) as f32,
            (t[1] - origin[1]) as f32,
            (t[2] - origin[2]) as f32,
        );
        let dist_sq = f64::from(delta.length_squared());
        if !(0.25..=AIM_MAX_DISTANCE * AIM_MAX_DISTANCE).contains(&dist_sq) {
            continue;
        }
        let dist = dist_sq.sqrt() as f32;
        let to = delta / dist;
        let cos = dir.dot(to);
        if cos < AIM_MAX_COS {
            continue;
        }
        let ticks = (dist / speed).min(40.0);
        let compensated = Vec3::new(
            delta.x,
            delta.y + 0.5 * ITEM_GRAVITY_BPT2 * ticks * ticks,
            delta.z,
        )
        .normalize_or_zero();
        if compensated.length_squared() > 1e-6 && best.is_none_or(|(d, _)| dist_sq < d) {
            best = Some((dist_sq, compensated));
        }
    }
    match best {
        Some((_, target_dir)) => dir.lerp(target_dir, 0.72).normalize_or_zero() * speed,
        None => vel,
    }
}

fn hand_world_velocity(data: &VrData, history: &super::vr_trackers::History) -> (Vec3, f32) {
    let room_delta = history.net_movement(0.12);
    let vel_mps = data.room.turn() * (room_delta * data.room.scale) / 0.12;
    let speed = history.average_speed(0.14) * data.room.scale;
    (vel_mps, speed)
}

impl Game<'_> {
    /// Registers an already-spawned dropped item id in `ThrowTracker` (used by both grip throws
    /// and boosted Drop key throws).
    pub(super) fn vr_register_thrown_item(
        &self,
        item_id: u32,
        hand: usize,
        stack: &horizon_client::items::ItemStack,
    ) {
        let loyalty = stack.enchantment_level("minecraft:loyalty");
        let (damage, sharp, boomerang) = weapon_throw_stats(&stack.item, loyalty);
        if let Some(vr) = vr_player().as_mut() {
            vr.trackers.throw.thrown.insert(
                item_id,
                ThrownMeta {
                    hand,
                    damage,
                    sharp,
                    boomerang: boomerang && vr_settings().throw_boomerang,
                    returning: false,
                    ticks: 0,
                    embedded: false,
                },
            );
        }
    }

    /// Computes throw spawn & velocity if hand `hand` is currently swinging faster than `min_speed`.
    pub(super) fn vr_hand_throw_impulse(
        &self,
        hand: usize,
        min_speed: f32,
    ) -> Option<([f64; 3], [f64; 3])> {
        let s = vr_settings();
        if !self.vr_running() || !s.vr_throwing {
            return None;
        }
        let poses = horizon_vr::poses()?;
        let guard = vr_player();
        let vr = guard.as_ref().filter(|v| v.placed)?;
        let data = vr.data(poses);
        let (hand_vel, speed) = hand_world_velocity(&data, &vr.trackers.controller_history[hand]);
        if speed < min_speed || hand_vel.length_squared() < 1e-4 {
            return None;
        }
        let pv = self.player.as_ref().map_or(Vec3::ZERO, |p| {
            Vec3::new(p.velocity.x as f32, p.velocity.y as f32, p.velocity.z as f32)
        });
        let origin = data.hand_pos(hand);
        let mut vel = compute_throw_velocity(hand_vel, pv, s.throw_velocity_mult);
        if s.throw_aim_assist {
            let riding = self.riding;
            let targets: Vec<[f64; 3]> = self
                .mobs
                .iter()
                .filter(|m| !m.removed && m.alive() && Some(m.id) != riding)
                .map(|m| {
                    let c = m.bounds().center();
                    [c.x, c.y, c.z]
                })
                .collect();
            vel = apply_aim_assist(origin, vel, &targets);
        }
        Some((
            origin,
            [f64::from(vel.x), f64::from(vel.y), f64::from(vel.z)],
        ))
    }

    /// Runs VR grip throw gestures, physical catching, and airborne thrown-weapon physics.
    pub(super) fn vr_throw_tracker(&mut self, data: VrData) {
        let s = vr_settings();
        let active = self.screen == Screen::Playing && self.sleeping.is_none() && !horizon_vr::gui_open();
        let pad = horizon_vr::pad();
        let now = Instant::now();
        let mut pending_throws: Vec<(usize, [f64; 3], Vec3)> = Vec::new();
        let mut catch_checks: Vec<(usize, [f64; 3])> = Vec::new();

        {
            let mut guard = vr_player();
            let Some(vr) = guard.as_mut() else {
                vr_throw_reset();
                return;
            };
            let pv = self.player.as_ref().map_or(Vec3::ZERO, |p| {
                Vec3::new(p.velocity.x as f32, p.velocity.y as f32, p.velocity.z as f32)
            });

            if s.vr_throwing && active && let Some(pad) = pad {
                let grips = [pad.right_bumper, pad.left_bumper];
                let busy = [
                    vr.trackers.climb.latched[0] || super::vr_camera::vr_camera_touched(),
                    vr.trackers.climb.latched[1],
                ];
                for c in 0..2 {
                    let down = grips[c] && !busy[c];
                    let has_item = match c {
                        0 => self.held().is_some(),
                        _ => self.offhand.is_some(),
                    };
                    let (hand_vel, speed) =
                        hand_world_velocity(&data, &vr.trackers.controller_history[c]);
                    let t = &mut vr.trackers.throw;

                    if down {
                        if !t.was_grip[c] {
                            t.hold_start[c] = Some(now);
                            t.armed[c] = false;
                        }
                        if has_item
                            && (t.hold_start[c].is_some_and(|st| now.duration_since(st).as_millis() >= HOLD_ARM_MS)
                                || speed >= MIN_THROW_SPEED_MPS)
                        {
                            t.armed[c] = true;
                        }
                        if s.throw_catch {
                            catch_checks.push((c, data.hand_pos(c)));
                        }
                    } else if t.was_grip[c] {
                        if t.armed[c] && has_item && speed >= MIN_THROW_SPEED_MPS {
                            let origin = data.hand_pos(c);
                            let vel = compute_throw_velocity(hand_vel, pv, s.throw_velocity_mult);
                            pending_throws.push((c, origin, vel));
                        }
                        t.hold_start[c] = None;
                        t.armed[c] = false;
                    } else {
                        t.hold_start[c] = None;
                        t.armed[c] = false;
                    }
                    t.was_grip[c] = down;
                }
                *GRIP_ARMED.lock().unwrap_or_else(|e| e.into_inner()) = vr.trackers.throw.armed;
            } else {
                vr.trackers.throw.hold_start = [None; 2];
                vr.trackers.throw.was_grip = [false; 2];
                vr.trackers.throw.armed = [false; 2];
                vr_throw_reset();
            }
        }

        // Physical catching with held grip.
        if s.throw_catch && !catch_checks.is_empty()
            && let Some(player_pos) = self.player.as_ref().map(|p| p.position)
        {
            let target_pos = WVec3::new(
                player_pos.x,
                player_pos.y + 0.5,
                player_pos.z,
            );
            let mut caught = Vec::new();
            for item in &mut self.dropped {
                let moving = !item.on_ground
                    || (item.velocity.x * item.velocity.x
                        + item.velocity.y * item.velocity.y
                        + item.velocity.z * item.velocity.z)
                        > 0.0025;
                if !moving {
                    continue;
                }
                for &(c, hand) in &catch_checks {
                    let dx = item.position.x - hand[0];
                    let dy = item.position.y - hand[1];
                    let dz = item.position.z - hand[2];
                    if dx * dx + dy * dy + dz * dz <= CATCH_RADIUS * CATCH_RADIUS {
                        item.pickup_delay = 0;
                        item.no_gravity = false;
                        item.position = target_pos;
                        item.velocity = WVec3::new(0.0, 0.0, 0.0);
                        item.needs_sync = true;
                        caught.push((c, item.id));
                        break;
                    }
                }
            }
            if !caught.is_empty() {
                if let Some(vr) = vr_player().as_mut() {
                    for &(c, id) in &caught {
                        vr.trackers.throw.thrown.remove(&id);
                        horizon_vr::haptic(c, 900);
                    }
                }
            }
        }

        // Execute any throw released this tick.
        if !pending_throws.is_empty() {
            let riding = self.riding;
            let targets: Vec<[f64; 3]> = if s.throw_aim_assist {
                self.mobs
                    .iter()
                    .filter(|m| !m.removed && m.alive() && Some(m.id) != riding)
                    .map(|m| {
                        let c = m.bounds().center();
                        [c.x, c.y, c.z]
                    })
                    .collect()
            } else {
                Vec::new()
            };
            for (c, origin, mut vel) in pending_throws {
                if s.throw_aim_assist {
                    vel = apply_aim_assist(origin, vel, &targets);
                }
                self.vr_execute_throw(c, s.throw_whole_stack, origin, vel);
            }
        }

        // Tick airborne thrown items for weapon impact, block embedding, and boomerang return.
        self.vr_tick_thrown_items(&data, s.throw_weapon_damage, s.throw_boomerang);
    }

    fn vr_execute_throw(&mut self, hand: usize, all: bool, pos: [f64; 3], vel: Vec3) {
        let spectator = self.spectator();
        let slot_stack = if hand == 0 {
            &mut self.inventory[self.selected]
        } else {
            &mut self.offhand
        };
        let Some(thrown) = horizon_client::items::take_held_drop(slot_stack, all, spectator) else {
            return;
        };
        let swing_hand = if hand == 0 { Hand::Main } else { Hand::Off };
        self.swing_hand(swing_hand, SwingAnimation::DEFAULT);

        if self.remote.is_some() {
            self.vr_net_active_hand(hand as u8, true);
            self.vr_net_aim_override(Some(pos), Some(vel));
            self.vr_net_aim_override_reset(1);
            // Tell remote server to drop if main hand. Note: offhand drops are client-local
            // unless swapped first, matching vanilla server action constraints.
            if hand == 0 {
                use horizon_protocol::packets::play::{PlayerAction, Serverbound as S};
                self.send(S::PlayerAction(PlayerAction {
                    action: if all { 4 } else { 5 },
                    pos: [0, 0, 0],
                    direction: 0,
                    sequence: 0,
                }));
            }
            return;
        }

        let uuid = self.acting_player_uuid();
        let v = [f64::from(vel.x), f64::from(vel.y), f64::from(vel.z)];
        let (item_name, count) = (thrown.item.clone(), thrown.count as i32);
        let stack_copy = thrown.clone();
        if let Some(id) = self.toss_item(thrown, pos, v, Some(uuid)) {
            if let Some(item) = self.dropped.iter_mut().rev().find(|d| d.id == id) {
                item.pickup_delay = 10;
                item.thrower = Some(uuid);
            }
            self.vr_register_thrown_item(id, hand, &stack_copy);
            self.award_stat("dropped", &item_name, count);
            self.award_custom_stat("drop", 1);
            let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
            self.audio.play("minecraft:entity.snowball.throw", Some(pos), 0.5, pitch);
            horizon_vr::haptic(hand, 1200);
        }
    }

    fn vr_tick_thrown_items(&mut self, data: &VrData, do_damage: bool, do_boomerang: bool) {
        let mut active: HashMap<u32, ThrownMeta> = match vr_player().as_mut() {
            Some(vr) => std::mem::take(&mut vr.trackers.throw.thrown),
            None => return,
        };
        if active.is_empty() {
            return;
        }
        let riding = self.riding;
        let h0 = data.hand_pos(0);
        let h1 = data.hand_pos(1);
        let from = self.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]);
        let mut hits: Vec<(u32, usize, f32, [f64; 3], usize)> = Vec::new();
        let mut embed_sounds: Vec<[f64; 3]> = Vec::new();

        for item in &mut self.dropped {
            let Some(meta) = active.get_mut(&item.id) else {
                continue;
            };
            meta.ticks += 1;
            if meta.ticks > 200 {
                item.no_gravity = false;
                continue;
            }
            let speed = (item.velocity.x * item.velocity.x
                + item.velocity.y * item.velocity.y
                + item.velocity.z * item.velocity.z)
                .sqrt();

            // Check impact with living mobs before block embedding.
            if do_damage && !meta.embedded && speed > 0.25 {
                let p = item.position;
                let probe = Aabb::new(
                    WVec3::new(p.x - 0.25, p.y - 0.25, p.z - 0.25),
                    WVec3::new(p.x + 0.25, p.y + 0.25, p.z + 0.25),
                );
                if let Some((mob_idx, _)) = self
                    .mobs
                    .iter()
                    .enumerate()
                    .find(|(_, m)| !m.removed && m.alive() && Some(m.id) != riding && m.bounds().intersects(probe))
                {
                    let dmg = meta.damage * ((speed as f32) / 0.8).clamp(0.5, 1.8);
                    hits.push((item.id, mob_idx, dmg, [p.x, p.y, p.z], meta.hand));
                    if meta.boomerang && do_boomerang {
                        meta.returning = true;
                    } else {
                        meta.embedded = true;
                        item.velocity = WVec3::new(
                            -item.velocity.x * 0.25,
                            -item.velocity.y * 0.25,
                            -item.velocity.z * 0.25,
                        );
                        item.needs_sync = true;
                    }
                    continue;
                }
            }

            // Sharp weapon/tool block embedding when stopped on a surface early in flight.
            if meta.sharp
                && !meta.returning
                && !meta.embedded
                && meta.ticks <= 60
                && (item.on_ground || item.horizontal_collision || item.vertical_collision)
            {
                meta.embedded = true;
                item.velocity = WVec3::new(0.0, 0.0, 0.0);
                item.needs_sync = true;
                embed_sounds.push([item.position.x, item.position.y, item.position.z]);
            }

            // Boomerang return flight back toward the throwing hand.
            if do_boomerang && meta.boomerang && (meta.returning || meta.ticks >= BOOMERANG_RETURN_TICKS) {
                meta.returning = true;
                meta.embedded = false;
                let target = if meta.hand == 0 { h0 } else { h1 };
                let dx = target[0] - item.position.x;
                let dy = target[1] - item.position.y;
                let dz = target[2] - item.position.z;
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                if dist <= 0.7 {
                    item.pickup_delay = 0;
                    item.no_gravity = false;
                } else {
                    let s = BOOMERANG_RETURN_SPEED / dist.max(1e-4);
                    item.no_gravity = true;
                    item.velocity = WVec3::new(dx * s, dy * s, dz * s);
                    item.needs_sync = true;
                }
            }
        }

        // Apply deferred mob hits with Horizon's `hurt_by_player`.
        for (item_id, mob_idx, dmg, pos, hand) in hits {
            if let Some(m) = self.mobs.get(mob_idx) && m.alive() && !m.removed {
                let source = match from {
                    Some(f) => self.attack_source(f),
                    None => horizon_client::mobs::Damage::of(
                        horizon_client::mobs::DamageKind::Generic,
                    ),
                };
                if self.hurt_by_player(mob_idx, dmg, source) {
                    self.audio.play("minecraft:entity.player.attack.strong", Some(pos), 0.9, 1.0);
                    horizon_vr::haptic(hand, 1000);
                }
            } else {
                active.remove(&item_id);
            }
        }
        for pos in embed_sounds {
            self.audio.play("minecraft:item.trident.hit_ground", Some(pos), 0.8, 1.1);
        }

        // Drop stale entries that left the world or timed out.
        active.retain(|id, meta| {
            meta.ticks <= 200 && self.dropped.iter().any(|d| d.id == *id)
        });
        if let Some(vr) = vr_player().as_mut() {
            vr.trackers.throw.thrown = active;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throw_velocity_scales_and_clamps() {
        let mild = compute_throw_velocity(Vec3::new(4.0, 2.0, 0.0), Vec3::new(0.1, 0.0, 0.0), 1.5);
        // 4.0 * 0.05 * 1.5 = 0.30 + 0.10 = 0.40
        assert!((mild.x - 0.4).abs() < 1e-5 && (mild.y - 0.15).abs() < 1e-5, "{mild}");

        let extreme = compute_throw_velocity(Vec3::new(100.0, 0.0, 0.0), Vec3::ZERO, 2.0);
        assert!((extreme.length() - MAX_THROW_SPEED_BPT).abs() < 1e-4, "{extreme}");
    }

    #[test]
    fn weapon_throw_classification_matches_item_kinds() {
        assert_eq!(weapon_throw_stats("minecraft:netherite_sword", 0), (8.0, true, true));
        assert_eq!(weapon_throw_stats("minecraft:diamond_axe", 0), (9.0, true, true));
        assert_eq!(weapon_throw_stats("minecraft:iron_pickaxe", 0), (4.5, true, true));
        assert_eq!(weapon_throw_stats("minecraft:trident", 0), (8.0, true, true));
        assert_eq!(weapon_throw_stats("minecraft:cobblestone", 0), (2.0, false, false));
        assert_eq!(weapon_throw_stats("minecraft:cobblestone", 1), (2.0, false, true));
    }

    #[test]
    fn aim_assist_steers_within_cone_and_ignores_outside() {
        let origin = [0.0, 1.5, 0.0];
        let vel = Vec3::new(0.0, 0.0, 1.0); // throwing toward +Z
        let target_in_cone = [0.5, 1.5, 10.0]; // ~2.9 deg off +Z
        let assisted = apply_aim_assist(origin, vel, &[target_in_cone]);
        assert!(assisted.x > 0.02, "should steer right toward target: {assisted}");
        assert!(assisted.y > 0.05, "should compensate gravity arc: {assisted}");
        assert!((assisted.length() - vel.length()).abs() < 1e-4);

        let target_behind = [0.0, 1.5, -10.0];
        let unassisted = apply_aim_assist(origin, vel, &[target_behind]);
        assert!((unassisted - vel).length() < 1e-5);
    }
}
