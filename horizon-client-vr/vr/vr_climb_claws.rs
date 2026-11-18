//! Vivecraft's climbey items, ported from VivecraftMod (LGPL-3.0, `data/ViveItems.java`,
//! `ClimbTracker` with claws, `JumpTracker` with boots). Climb Claws (unbreakable shears named
//! "Climb Claws") grab any solid block with the trigger; Jump Boots (unbreakable leather boots
//! named "Jump Boots") jump by grabbing with a grip, pulling down and letting go. Vivecraft needs
//! its server for both (`SERVER_ALLOWS_CLIMBEY`): here, the world's host is this game, so in single
//! player and a hosted LAN world.
use super::Game;
use super::vr_player::vr_player;
use glam::Vec3;
use horizon_client::items::ItemStack;
use std::sync::Mutex;

fn named(stack: &ItemStack, item: &str, name: &str, key: &str) -> bool {
    stack.item.as_ref() == item
        && stack
            .extra
            .as_ref()
            .is_some_and(|d| d.other("minecraft:unbreakable").is_some())
        && stack.custom_name().is_some_and(|n| n == name || n == key)
}

/// `ViveItems.isClimbingClaws`.
pub(crate) fn is_climb_claws(stack: &ItemStack) -> bool {
    named(stack, "minecraft:shears", "Climb Claws", "vivecraft.item.climbclaws")
}

/// `ViveItems.isJumpBoots`.
pub(crate) fn is_jump_boots(stack: &ItemStack) -> bool {
    named(stack, "minecraft:leather_boots", "Jump Boots", "vivecraft.item.jumpboots")
}

impl Game<'_> {
    /// The world's host allows the climbey items (`SERVER_ALLOWS_CLIMBEY`).
    // The integrated server trusts its owner's moves, as Vivecraft's server does with climbey on.
    fn vr_climbey_allowed(&self) -> bool {
        self.has_singleplayer_server() || super::vr_net::server_allows_climbey()
    }

    /// Climb claws in hand `c` (0 main, 1 offhand).
    pub(super) fn vr_claws_in(&self, c: usize) -> bool {
        let stack = if c == 0 { self.held() } else { self.offhand.as_ref() };
        self.vr_climbey_allowed() && stack.is_some_and(is_climb_claws)
    }

    /// `hasClimbeyJumpEquipped`.
    pub(super) fn vr_jump_boots(&self) -> bool {
        self.vr_climbey_allowed() && self.armor[3].as_ref().is_some_and(is_jump_boots)
    }
}

/// `hasClimbeyClimbEquipped`.
pub(super) fn vr_claws_held(game: &Game<'_>) -> bool {
    game.vr_claws_in(0) || game.vr_claws_in(1)
}

/// `JumpTracker`'s climbey jump: latched by the grips, where the hands were then (in the room).
#[derive(Default)]
struct Boots {
    latched: [bool; 2],
    start: Vec3,
}

static BOOTS: Mutex<Option<Boots>> = Mutex::new(None);

/// Drops grip latches when VR stops, so the next session starts from the current grips.
pub(crate) fn vr_boots_reset() {
    *BOOTS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The boots take the grips (`keyClimbeyJump`) while they are worn on the ground.
pub(crate) fn vr_boots_take_grips() -> bool {
    BOOTS.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

impl Game<'_> {
    /// `JumpTracker.activeProcess` with jump boots: grip, pull the hands down, let go to jump
    /// the other way. Returns whether the boots are worn (the head-rise jump is off then, as
    /// `realisticJumpEnabled` AUTO).
    // shortcut: the room doesn't follow the pull while latched (Vivecraft's crouch feel).
    pub(super) fn vr_boots_jump(&mut self) -> bool {
        let on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
        if !self.vr_jump_boots() || !super::vr_settings::vr_settings().jump {
            *BOOTS.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return false;
        }
        let Some(pad) = horizon_vr::pad() else {
            return true;
        };
        let guard = vr_player();
        let Some(vr) = guard.as_ref() else {
            return true;
        };
        let Some(data) = vr.pre else {
            return true;
        };
        let room = |c: usize| data.poses.aim[c].map_or(Vec3::ZERO, |p| p.translation());
        let now = room(0).lerp(room(1), 0.5);
        let h = &vr.trackers.controller_history;
        let movement = h[0].net_movement(0.3) + h[1].net_movement(0.3);
        let speed = h[0].average_speed(0.3) + h[1].average_speed(0.3) * 0.5;
        drop(guard);
        let mut lock = BOOTS.lock().unwrap_or_else(|e| e.into_inner());
        let b = lock.get_or_insert_with(Default::default);
        let ok = pad.left_bumper || pad.right_bumper;
        let mut jump = false;
        for c in 0..2 {
            if !ok && b.latched[c] {
                horizon_vr::haptic(c, 200);
                jump = true;
            }
            if ok && !b.latched[c] {
                b.start = now;
                horizon_vr::haptic(c, 1000);
            }
            b.latched[c] = ok && on_ground;
        }
        let delta = now - b.start;
        drop(lock);
        if jump {
            self.vr_net_jumping();
            let mut m = movement * (0.33 * speed);
            if m.length() > 0.66 {
                m *= 0.66 / m.length();
            }
            let m = data.room.dir_to_world(m);
            if delta.y < 0.0 && m.y < 0.0 {
                if let Some(p) = self.player.as_mut() {
                    p.velocity.x -= f64::from(m.x) * 1.25;
                    p.velocity.y = -f64::from(m.y);
                    p.velocity.z -= f64::from(m.z) * 1.25;
                    p.ground.on_ground = false;
                }
            }
        }
        true
    }
}
