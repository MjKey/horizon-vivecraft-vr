//! Vivecraft's boat controls in standing VR, ported from VivecraftMod (LGPL-3.0,
//! https://github.com/Vivecraft/VivecraftMod): `AbstractBoatMixin` (stick turning, tank controls
//! with `vehicleRotation`) and `RowTracker` (`realisticRowEnabled`: swing both arms to row).
use super::vr_player::{vr_player, wrap_degrees, yaw_of};

/// `RowTracker.activeProcess`'s oars: right, left, forward.
fn oars() -> [f32; 3] {
    const MIN_SPEED: f32 = 0.5;
    const MAX_SPEED: f32 = 2.0;
    let guard = vr_player();
    let Some(vr) = guard.as_ref() else {
        return [0.0; 3];
    };
    // RowTracker.isActive: `realisticRowEnabled`, no rowing while the bow is notched.
    if !super::vr_settings::vr_settings().row || vr.trackers.bow.is_notched() {
        return [0.0; 3];
    }
    let h = &vr.trackers.controller_history;
    let r = (h[0].average_speed(0.5) - MIN_SPEED).max(0.0);
    let l = (h[1].average_speed(0.5) - MIN_SPEED).max(0.0);
    let f = if r > 0.0 && l > 0.0 { (r + l) / 2.0 } else { 0.0 };
    [r.min(MAX_SPEED), l.min(MAX_SPEED), f.min(MAX_SPEED)]
}

/// `controlBoat` in VR: the boat's inputs `[left, right, up, down]`, its yaw, and its
/// `delta_rotation` and `acceleration` as vanilla left them before the push; returns the
/// acceleration and the inputs (for the paddles) Vivecraft leaves.
pub(crate) fn vr_control_boat(
    input: [bool; 4],
    boat_yaw: f32,
    delta_rotation: &mut f32,
    acceleration: f32,
) -> (f32, [bool; 4]) {
    let Some(pad) = horizon_vr::pad() else {
        return (acceleration, input);
    };
    let [mut left, mut right, mut up, down] = input;
    // `vivecraft$inputLeft/Right`: turning by the stick's amount, not 1.
    let move_x = -pad.left[0];
    if left {
        *delta_rotation += 1.0 - move_x;
    }
    if right {
        *delta_rotation += -move_x - 1.0;
    }
    let mut acceleration = acceleration;
    if up {
        // The offhand controller's yaw (`FreeMove.CONTROLLER`), tank controls.
        let Some(yaw) = vr_player()
            .as_ref()
            .and_then(|v| v.pre)
            .map(|d| yaw_of(d.controller_dir(1)))
        else {
            return (acceleration, input);
        };
        let difference = wrap_degrees(boat_yaw - yaw);
        acceleration = if difference.abs() < 30.0 {
            0.04
        } else if difference.abs() > 150.0 {
            -0.005
        } else if difference != 0.0 {
            0.005
        } else {
            0.0
        };
        if difference.abs() > 10.0 && difference.abs() < 150.0 {
            *delta_rotation -= difference.signum() * ((difference.abs() - 25.0).max(0.0) / 40.0).min(1.0);
        }
    } else {
        let [r, l, f] = oars();
        if r + l + f > 0.0 {
            *delta_rotation += l / 1.5;
            *delta_rotation -= r / 1.5;
            if *delta_rotation < 0.0 {
                left = true;
            }
            if *delta_rotation > 0.0 {
                right = true;
            }
            acceleration = (0.06 * f).min(0.04);
            if acceleration > 0.0 {
                up = true;
            }
        }
    }
    (acceleration, [left, right, up, down])
}
