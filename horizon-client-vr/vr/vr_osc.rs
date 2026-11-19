//! Vivecraft's `OSCTrackerReceiver`, ported from VivecraftMod (LGPL-3.0,
//! `client_vr/utils/osc_trackers/OSCTrackerReceiver.java`): full-body trackers (SlimeVR and other
//! apps that speak VRChat's OSC tracker spec) sent to UDP port 9000, as `/tracking/trackers/N/position`
//! and `/rotation` (N 1 to 8, `head` for the alignment). Off unless the "OSC Trackers" VR setting is on.
//! The waist and feet go to a Vivecraft server with the player's tracking (`FBTMode.ARMS_LEGS`).
// shortcut: trackers get their roles (two lowest the feet, the next the waist) by height instead of
// Vivecraft's T-pose calibration screen, and the head's turn isn't smoothed; add the calibration if
// trackers come out swapped.
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `oscTrackerPort`.
const PORT: u16 = 9000;
/// `OSCTracker.TIMEOUT`.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Default)]
struct Raw {
    position: Option<([f32; 3], Instant)>,
    rotation: Option<([f32; 3], Instant)>,
}

/// 0 the head, 1..=8 the trackers.
static TRACKERS: Mutex<[Raw; 9]> = Mutex::new([Raw { position: None, rotation: None }; 9]);
static STARTED: std::sync::Once = std::sync::Once::new();

/// One OSC message: its address and float arguments.
fn message(b: &[u8]) -> Option<(&str, Vec<f32>)> {
    let padded = |n: usize| (n + 4) & !3;
    let end = b.iter().position(|&c| c == 0)?;
    let address = std::str::from_utf8(&b[..end]).ok()?;
    let b = b.get(padded(end)..)?;
    let tags_end = b.iter().position(|&c| c == 0)?;
    let tags = b.get(..tags_end)?.strip_prefix(b",")?;
    let mut rest = b.get(padded(tags_end)..)?;
    let mut args = Vec::new();
    for &t in tags {
        let (word, more) = (rest.get(..4)?, rest.get(4..)?);
        let bytes: [u8; 4] = word.try_into().ok()?;
        args.push(match t {
            b'f' => f32::from_be_bytes(bytes),
            b'i' => i32::from_be_bytes(bytes) as f32,
            _ => return None,
        });
        rest = more;
    }
    Some((address, args))
}

/// The messages of a packet: one message or a `#bundle` of them (bundles nest).
fn messages<'a>(b: &'a [u8], out: &mut Vec<(&'a str, Vec<f32>)>) {
    if let Some(mut rest) = b.strip_prefix(b"#bundle\0").and_then(|r| r.get(8..)) {
        while rest.len() >= 4 {
            let (size, after) = rest.split_at(4);
            let n = u32::from_be_bytes(size.try_into().expect("4 bytes")) as usize;
            if n > after.len() {
                return;
            }
            let (element, more) = after.split_at(n);
            messages(element, out);
            rest = more;
        }
    } else if let Some(m) = message(b) {
        out.push(m);
    }
}

/// `getMessageTrackerIndex` and the kind: `/tracking/trackers/<head|1..8>/<position|rotation>`.
fn store(address: &str, args: &[f32], now: Instant) {
    let Some(rest) = address.strip_prefix("/tracking/trackers/") else {
        return;
    };
    let Some((who, kind)) = rest.split_once('/') else {
        return;
    };
    let index = match who {
        "head" => 0,
        n => match n.parse::<usize>() {
            Ok(i @ 1..=8) => i,
            _ => return,
        },
    };
    let Some(v) = args.get(..3).map(|a| [a[0], a[1], a[2]]).filter(|v| v.iter().all(|x| x.is_finite())) else {
        return;
    };
    let mut t = TRACKERS.lock().unwrap_or_else(|e| e.into_inner());
    match kind {
        "position" => t[index].position = Some((v, now)),
        "rotation" => t[index].rotation = Some((v, now)),
        _ => {}
    }
}

/// Starts listening (once) when the setting is on.
fn listen() {
    STARTED.call_once(|| {
        let socket = match std::net::UdpSocket::bind(("127.0.0.1", PORT)) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Horizon VR: OSC trackers can't listen on port {PORT}: {e}");
                return;
            }
        };
        log::info!("Horizon VR: listening for OSC trackers on port {PORT}");
        let spawned = std::thread::Builder::new().name("vr-osc".into()).spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = socket.recv(&mut buf) {
                let mut out = Vec::new();
                messages(&buf[..n], &mut out);
                let now = Instant::now();
                for (address, args) in out {
                    store(address, &args, now);
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("Horizon VR: OSC trackers: {e}");
        }
    });
}

/// VRChat's tracker rotation (degrees, Unity's axes) as Vivecraft turns it:
/// `rotationZ(-z).rotateX(x).rotateY(y)`.
fn turn(r: [f32; 3]) -> glam::Quat {
    glam::Quat::from_rotation_z(-r[2].to_radians())
        * glam::Quat::from_rotation_x(r[0].to_radians())
        * glam::Quat::from_rotation_y(r[1].to_radians())
}

/// The waist, right foot and left foot in the room (position, rotation), when the setting is on
/// and at least three trackers send. `hmd` is the headset in the room.
pub(crate) fn osc_body(hmd: horizon_vr::Pose) -> Option<[(glam::Vec3, glam::Quat); 3]> {
    if !super::vr_settings::vr_settings().osc_trackers {
        return None;
    }
    listen();
    let t = *TRACKERS.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = |x: Option<([f32; 3], Instant)>| x.filter(|(_, at)| at.elapsed() < TIMEOUT).map(|(v, _)| v);
    // The head's message aligns the trackers' space with the room's.
    let offset = fresh(t[0].position).map_or(glam::Vec3::ZERO, |h| glam::Vec3::new(-h[0], -h[1], h[2]));
    let (hmd_rot, hmd_pos) = (hmd.rotation(), hmd.translation());
    let hmd_dir = hmd_rot * glam::Vec3::NEG_Z;
    let rotation_offset = fresh(t[0].rotation).map_or(0.0, |r| {
        let fwd = turn(r) * glam::Vec3::NEG_Z;
        (-fwd.x).atan2(fwd.z) - (-hmd_dir.x).atan2(hmd_dir.z)
    });
    let room_turn = glam::Quat::from_rotation_y(rotation_offset);
    // `hmdPivotHistory.latest()`: 10 cm below and behind the eyes.
    let pivot = hmd_pos + hmd_rot * glam::Vec3::new(0.0, -0.1, 0.1);
    let mut body: Vec<(glam::Vec3, glam::Quat)> = t[1..]
        .iter()
        .filter_map(|r| {
            let p = fresh(r.position)?;
            let q = fresh(r.rotation).map_or(glam::Quat::IDENTITY, turn);
            let at = room_turn * (glam::Vec3::new(p[0], p[1], -p[2]) + offset) + pivot;
            Some((at, (room_turn * q).normalize()))
        })
        .collect();
    if body.len() < 3 {
        return None;
    }
    body.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
    let waist = body[2];
    // Which foot is which: the headset's right (its yaw only).
    let right = glam::Vec3::new(-hmd_dir.z, 0.0, hmd_dir.x);
    let (a, b) = (body[0], body[1]);
    let (r, l) = if (a.0 - b.0).dot(right) >= 0.0 { (a, b) } else { (b, a) };
    Some([waist, r, l])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osc(address: &str, args: &[f32]) -> Vec<u8> {
        let mut b = address.as_bytes().to_vec();
        b.push(0);
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b.push(b',');
        b.extend(std::iter::repeat_n(b'f', args.len()));
        b.push(0);
        while b.len() % 4 != 0 {
            b.push(0);
        }
        for a in args {
            b.extend_from_slice(&a.to_be_bytes());
        }
        b
    }

    #[test]
    fn reads_messages_and_bundles() {
        let one = osc("/tracking/trackers/1/position", &[1.0, 2.0, 3.0]);
        let (address, args) = message(&one).expect("message");
        assert_eq!((address, args.as_slice()), ("/tracking/trackers/1/position", &[1.0, 2.0, 3.0][..]));
        let mut bundle = b"#bundle\0".to_vec();
        bundle.extend_from_slice(&[0; 8]);
        for m in [&one, &osc("/tracking/trackers/head/rotation", &[0.0, 90.0, 0.0])] {
            bundle.extend_from_slice(&(m.len() as u32).to_be_bytes());
            bundle.extend_from_slice(m);
        }
        let mut out = Vec::new();
        messages(&bundle, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].0, "/tracking/trackers/head/rotation");
    }
}
