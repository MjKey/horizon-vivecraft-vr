//! Vivecraft's VR settings (`VRSettings`) the port follows, with Vivecraft's defaults, saved in
//! `horizon-vr.txt` and set on the "VR Settings" page of the pause menu (Vivecraft's
//! `GuiMainVRSettings` / `GuiStandingSettings` / `GuiRoomscaleSettings` / `GuiHUDSettings` /
//! `GuiRadialConfiguration` rows the port has). From VivecraftMod (LGPL-3.0).
use horizon_hooks::pages::{Act, Label, Page, Place, Row};
use horizon_hooks::settings::Store;
use std::sync::Mutex;

/// `VRSettings.FreeMove` (the ones a standing player picks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FreeMove {
    Controller,
    Hmd,
    RunInPlace,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VrSettings {
    pub free_move: FreeMove,
    /// `worldRotationIncrement`: degrees per snap, 0 smooth.
    pub rotation_increment: f32,
    pub world_scale: f32,
    /// Teleporting with the right stick at all, and `vrLimitedSurvivalTeleport`.
    pub teleport: bool,
    pub limited_teleport: bool,
    pub jump: bool,
    pub sneak: bool,
    pub climb: bool,
    pub swim: bool,
    pub row: bool,
    pub crawl: bool,
    /// `vrHudLockMode`: the wrist, else the head.
    pub hud_head: bool,
    pub radial_hold: bool,
    pub radial_repeat: bool,
    pub walk_up_blocks: bool,
    pub simulate_falling: bool,
    pub camera_fov: f32,
    /// `seated`: no roomscale; the room stays on the player, walking goes where the head looks.
    pub seated: bool,
    /// `reverseHands`: left-handed.
    pub reverse_hands: bool,
    /// `displayMirrorMode`: what the desktop window shows.
    pub mirror: Mirror,
    /// Full-body trackers sent over OSC (`OSCTrackerReceiver`, port `oscTrackerPort` 9000).
    pub osc_trackers: bool,
    /// `renderDebugDeviceAxes` / `renderDebugPlayerAxes` (`DebugRenderHelper`).
    pub debug_device_axes: bool,
    pub debug_player_axes: bool,
}

/// `VRSettings.MirrorMode` (the ones the port draws).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mirror {
    /// `OFF`'s neighbour Horizon draws anyway: the game's own view (the HUD over the world when
    /// no headset HUD is wanted).
    FirstPerson,
    /// `THIRD_PERSON`: the handheld camera's picture, while it is out.
    ThirdPerson,
    /// `MIXED_REALITY`: the camera's picture, and beside it its foreground on green (for keying).
    MixedReality,
}

impl Default for VrSettings {
    fn default() -> Self {
        Self {
            free_move: FreeMove::Controller,
            rotation_increment: 45.0,
            world_scale: 1.0,
            teleport: true,
            limited_teleport: true,
            jump: true,
            sneak: true,
            climb: true,
            swim: true,
            row: true,
            crawl: true,
            hud_head: false,
            radial_hold: true,
            radial_repeat: true,
            walk_up_blocks: true,
            simulate_falling: true,
            camera_fov: 70.0,
            seated: false,
            reverse_hands: false,
            mirror: Mirror::FirstPerson,
            osc_trackers: false,
            debug_device_axes: false,
            debug_player_axes: false,
        }
    }
}

static SETTINGS: Mutex<Option<VrSettings>> = Mutex::new(None);

/// The settings now.
pub(crate) fn vr_settings() -> VrSettings {
    SETTINGS.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or_default()
}

fn change(f: impl FnOnce(&mut VrSettings)) {
    let mut lock = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = lock.unwrap_or_default();
    f(&mut s);
    *lock = Some(s);
    drop(lock);
    push(s);
}

/// What the `horizon_vr` crate reads itself.
fn push(s: VrSettings) {
    // Seated keeps the HUD where the player chose (wrist by default), not in the middle of the view.
    horizon_vr::set_hud_head(s.hud_head);
    horizon_vr::set_reverse_hands(s.reverse_hands);
    horizon_vr::set_mirror_mode(match s.mirror {
        Mirror::FirstPerson => 0,
        Mirror::ThirdPerson => 1,
        Mirror::MixedReality => 2,
    });
    horizon_vr::set_camera_fov(s.camera_fov);
}

const INCREMENTS: [f32; 6] = [0.0, 10.0, 30.0, 45.0, 90.0, 180.0];
const FILE: &str = "horizon-vr.txt";
const PREFIX: &str = "vr.";

pub(crate) static STORE: Lines = Lines;
pub(crate) struct Lines;

fn on(v: &str) -> Option<bool> {
    v.parse().ok()
}

impl Store for Lines {
    fn file(&self) -> &'static str {
        FILE
    }
    fn owns(&self, key: &str) -> bool {
        key.starts_with(PREFIX)
    }
    fn load(&self, key: &str, value: &str) {
        let key = &key[PREFIX.len()..];
        change(|s| {
            let flag = |b: &mut bool| {
                if let Some(v) = on(value) {
                    *b = v;
                }
            };
            match key {
                "freeMove" => {
                    s.free_move = match value {
                        "HMD" => FreeMove::Hmd,
                        "RUN_IN_PLACE" => FreeMove::RunInPlace,
                        _ => FreeMove::Controller,
                    }
                }
                "worldRotationIncrement" => {
                    if let Ok(v) = value.parse::<f32>()
                        && INCREMENTS.contains(&v)
                    {
                        s.rotation_increment = v;
                    }
                }
                "worldScale" => {
                    if let Ok(v) = value.parse::<f32>() {
                        s.world_scale = v.clamp(0.1, 100.0);
                    }
                }
                "handCameraFov" => {
                    if let Ok(v) = value.parse::<f32>() {
                        s.camera_fov = v.clamp(1.0, 179.0);
                    }
                }
                "teleport" => flag(&mut s.teleport),
                "limitedTeleport" => flag(&mut s.limited_teleport),
                "realisticJump" => flag(&mut s.jump),
                "realisticSneak" => flag(&mut s.sneak),
                "realisticClimb" => flag(&mut s.climb),
                "realisticSwim" => flag(&mut s.swim),
                "realisticRow" => flag(&mut s.row),
                "allowCrawling" => flag(&mut s.crawl),
                "hudHead" => flag(&mut s.hud_head),
                "radialModeHold" => flag(&mut s.radial_hold),
                "radialRepeat" => flag(&mut s.radial_repeat),
                "walkUpBlocks" => flag(&mut s.walk_up_blocks),
                "simulateFalling" => flag(&mut s.simulate_falling),
                "seated" => flag(&mut s.seated),
                "reverseHands" => flag(&mut s.reverse_hands),
                "oscTrackers" => flag(&mut s.osc_trackers),
                "renderDebugDeviceAxes" => flag(&mut s.debug_device_axes),
                "renderDebugPlayerAxes" => flag(&mut s.debug_player_axes),
                "displayMirrorMode" => {
                    s.mirror = match value {
                        "THIRD_PERSON" => Mirror::ThirdPerson,
                        "MIXED_REALITY" => Mirror::MixedReality,
                        _ => Mirror::FirstPerson,
                    }
                }
                _ => {}
            }
        });
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        let s = vr_settings();
        let b = |v: bool| if v { "true" } else { "false" };
        let free = match s.free_move {
            FreeMove::Controller => "CONTROLLER",
            FreeMove::Hmd => "HMD",
            FreeMove::RunInPlace => "RUN_IN_PLACE",
        };
        line("vr.freeMove", free);
        line("vr.worldRotationIncrement", &s.rotation_increment.to_string());
        line("vr.worldScale", &s.world_scale.to_string());
        line("vr.handCameraFov", &s.camera_fov.to_string());
        line(
            "vr.displayMirrorMode",
            match s.mirror {
                Mirror::FirstPerson => "FIRST_PERSON",
                Mirror::ThirdPerson => "THIRD_PERSON",
                Mirror::MixedReality => "MIXED_REALITY",
            },
        );
        for (k, v) in [
            ("vr.teleport", s.teleport),
            ("vr.limitedTeleport", s.limited_teleport),
            ("vr.realisticJump", s.jump),
            ("vr.realisticSneak", s.sneak),
            ("vr.realisticClimb", s.climb),
            ("vr.realisticSwim", s.swim),
            ("vr.realisticRow", s.row),
            ("vr.allowCrawling", s.crawl),
            ("vr.hudHead", s.hud_head),
            ("vr.radialModeHold", s.radial_hold),
            ("vr.radialRepeat", s.radial_repeat),
            ("vr.walkUpBlocks", s.walk_up_blocks),
            ("vr.simulateFalling", s.simulate_falling),
            ("vr.seated", s.seated),
            ("vr.reverseHands", s.reverse_hands),
            ("vr.oscTrackers", s.osc_trackers),
            ("vr.renderDebugDeviceAxes", s.debug_device_axes),
            ("vr.renderDebugPlayerAxes", s.debug_player_axes),
        ] {
            line(k, b(v));
        }
    }
    fn is_default(&self) -> bool {
        vr_settings() == VrSettings::default()
    }
}

/// The page's rows, in order.
const ROWS: [&str; 23] = [
    "vivecraft.options.FREEMOVE_MODE",
    "vivecraft.options.WORLD_ROTATION_INCREMENT",
    "vivecraft.options.WORLD_SCALE",
    "vivecraft.options.TELEPORT",
    "vivecraft.options.LIMIT_TELEPORT",
    "vivecraft.options.REALISTIC_JUMP",
    "vivecraft.options.REALISTIC_SNEAK",
    "vivecraft.options.REALISTIC_CLIMB",
    "vivecraft.options.REALISTIC_SWIM",
    "vivecraft.options.REALISTIC_ROW",
    "vivecraft.options.ALLOW_CRAWLING",
    "vivecraft.options.HUD_LOCK_TO",
    "vivecraft.options.RADIAL_MODE",
    "vivecraft.options.RADIAL_REPEAT",
    "vivecraft.options.WALK_UP_BLOCKS",
    "vivecraft.options.SIMULATE_FALLING",
    "vivecraft.options.HANDHELD_CAMERA_FOV",
    "vivecraft.options.SEATED",
    "vivecraft.options.REVERSE_HANDS",
    "vivecraft.options.MIRROR_DISPLAY",
    "vivecraft.options.OSC_TRACKERS",
    "vivecraft.options.RENDER_DEBUG_DEVICE_AXES",
    "vivecraft.options.RENDER_DEBUG_PLAYER_AXES",
];

pub(crate) static PAGE: VrPage = VrPage;
pub(crate) struct VrPage;

fn onoff(v: bool) -> Label {
    Label::Key(if v { "options.on" } else { "options.off" })
}

/// A world scale slider position (0..1) as the scale: 0.1 to 100, logarithmic.
fn scale_of(t: f32) -> f32 {
    let s = 10f32.powf(t * 3.0 - 1.0);
    (s * 100.0).round() / 100.0
}

fn slider_of_scale(s: f32) -> f32 {
    ((s.clamp(0.1, 100.0).log10() + 1.0) / 3.0).clamp(0.0, 1.0)
}

impl Page for VrPage {
    fn id(&self) -> &'static str {
        "horizon-vr.settings"
    }
    fn title(&self) -> &'static str {
        "vivecraft.options.screen.main"
    }
    fn place(&self) -> Place {
        Place::Pause
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        let s = vr_settings();
        let free = match s.free_move {
            FreeMove::Controller => "vivecraft.options.freemove.controller",
            FreeMove::Hmd => "vivecraft.options.freemove.hmd",
            FreeMove::RunInPlace => "vivecraft.options.freemove.runinplace",
        };
        let inc = if s.rotation_increment == 0.0 {
            Label::Key("vivecraft.options.rotationincrement.smooth")
        } else {
            Label::Value(format!("{}°", s.rotation_increment))
        };
        for (i, label) in ROWS.iter().copied().enumerate() {
            rows.push(match i {
                0 => Row::Cycle { label, value: Label::Key(free) },
                1 => Row::Cycle { label, value: inc.clone() },
                2 => Row::Slider {
                    label,
                    value: slider_of_scale(s.world_scale),
                    text: Label::Value(format!("{:.2}x", s.world_scale)),
                },
                11 => Row::Cycle {
                    label,
                    value: Label::Key(if s.hud_head { "vivecraft.options.hudlock.head" } else { "vivecraft.options.hudlock.wrist" }),
                },
                12 => Row::Cycle {
                    label,
                    value: Label::Key(if s.radial_hold { "vivecraft.options.radialmode.hold" } else { "vivecraft.options.radialmode.toggle" }),
                },
                16 => Row::Slider {
                    label,
                    value: (s.camera_fov - 30.0) / 80.0,
                    text: Label::Value(format!("{:.0}°", s.camera_fov)),
                },
                19 => Row::Cycle {
                    label,
                    value: Label::Key(match s.mirror {
                        Mirror::FirstPerson => "vivecraft.options.mirror.first_person",
                        Mirror::ThirdPerson => "vivecraft.options.mirror.third_person",
                        Mirror::MixedReality => "vivecraft.options.mirror.mixed_reality",
                    }),
                },
                _ => Row::Cycle { label, value: onoff(self.flag(&s, i)) },
            });
        }
    }
    fn act(&self, row: usize, act: Act) {
        let back = matches!(act, Act::PressBack);
        change(|s| match (row, act) {
            (2, Act::Slide(t)) => s.world_scale = scale_of(t),
            (16, Act::Slide(t)) => s.camera_fov = (30.0 + t.clamp(0.0, 1.0) * 80.0).round(),
            (0, Act::Press | Act::PressBack) => {
                let all = [FreeMove::Controller, FreeMove::Hmd, FreeMove::RunInPlace];
                let i = all.iter().position(|m| *m == s.free_move).unwrap_or(0);
                s.free_move = all[(i + if back { 2 } else { 1 }) % 3];
            }
            (1, Act::Press | Act::PressBack) => {
                let n = INCREMENTS.len();
                let i = INCREMENTS.iter().position(|v| *v == s.rotation_increment).unwrap_or(3);
                s.rotation_increment = INCREMENTS[(i + if back { n - 1 } else { 1 }) % n];
            }
            (11, Act::Press | Act::PressBack) => s.hud_head = !s.hud_head,
            (12, Act::Press | Act::PressBack) => s.radial_hold = !s.radial_hold,
            (19, Act::Press | Act::PressBack) => {
                let all = [Mirror::FirstPerson, Mirror::ThirdPerson, Mirror::MixedReality];
                let i = all.iter().position(|m| *m == s.mirror).unwrap_or(0);
                s.mirror = all[(i + if back { 2 } else { 1 }) % 3];
            }
            (i, Act::Press | Act::PressBack) => {
                if let Some(b) = Self::flag_mut(s, i) {
                    *b = !*b;
                }
            }
            _ => {}
        });
    }
}

impl VrPage {
    fn flag(&self, s: &VrSettings, i: usize) -> bool {
        let mut copy = *s;
        Self::flag_mut(&mut copy, i).is_some_and(|b| *b)
    }
    fn flag_mut(s: &mut VrSettings, i: usize) -> Option<&mut bool> {
        Some(match i {
            3 => &mut s.teleport,
            4 => &mut s.limited_teleport,
            5 => &mut s.jump,
            6 => &mut s.sneak,
            7 => &mut s.climb,
            8 => &mut s.swim,
            9 => &mut s.row,
            10 => &mut s.crawl,
            13 => &mut s.radial_repeat,
            14 => &mut s.walk_up_blocks,
            15 => &mut s.simulate_falling,
            17 => &mut s.seated,
            18 => &mut s.reverse_hands,
            20 => &mut s.osc_trackers,
            21 => &mut s.debug_device_axes,
            22 => &mut s.debug_player_axes,
            _ => return None,
        })
    }
}

/// Vivecraft's English names for the rows (its `assets/vivecraft/lang/en_us.json` wording).
static LANG: &horizon_hooks::lang::Files = &[("en_us", r#"{
"vivecraft.options.screen.main": "VR Settings",
"vivecraft.options.FREEMOVE_MODE": "Free Move Type",
"vivecraft.options.freemove.controller": "Controller",
"vivecraft.options.freemove.hmd": "HMD",
"vivecraft.options.freemove.runinplace": "Run In Place",
"vivecraft.options.WORLD_ROTATION_INCREMENT": "Rotation Increment",
"vivecraft.options.rotationincrement.smooth": "Smooth",
"vivecraft.options.WORLD_SCALE": "World Scale",
"vivecraft.options.TELEPORT": "Teleport",
"vivecraft.options.LIMIT_TELEPORT": "Limit in Survival",
"vivecraft.options.REALISTIC_JUMP": "Roomscale Jumping",
"vivecraft.options.REALISTIC_SNEAK": "Roomscale Sneaking",
"vivecraft.options.REALISTIC_CLIMB": "Roomscale Climbing",
"vivecraft.options.REALISTIC_SWIM": "Roomscale Swimming",
"vivecraft.options.REALISTIC_ROW": "Roomscale Rowing",
"vivecraft.options.ALLOW_CRAWLING": "Roomscale Crawling",
"vivecraft.options.HUD_LOCK_TO": "HUD Locked To",
"vivecraft.options.hudlock.wrist": "Wrist",
"vivecraft.options.hudlock.head": "Head",
"vivecraft.options.RADIAL_MODE": "Radial Menu Mode",
"vivecraft.options.radialmode.hold": "Hold",
"vivecraft.options.radialmode.toggle": "Press",
"vivecraft.options.RADIAL_REPEAT": "Radial Repeat",
"vivecraft.options.WALK_UP_BLOCKS": "Walk Up Blocks",
"vivecraft.options.SIMULATE_FALLING": "Simulate Falling",
"vivecraft.options.HANDHELD_CAMERA_FOV": "Camera FOV",
"vivecraft.options.SEATED": "Seated Mode",
"vivecraft.options.REVERSE_HANDS": "Reverse Hands",
"vivecraft.options.MIRROR_DISPLAY": "Desktop Mirror",
"vivecraft.options.mirror.first_person": "First Person",
"vivecraft.options.mirror.third_person": "Third Person (Camera)",
"vivecraft.options.mirror.mixed_reality": "Mixed Reality",
"vivecraft.options.OSC_TRACKERS": "OSC Trackers (port 9000)",
"vivecraft.options.RENDER_DEBUG_DEVICE_AXES": "Local Device Axes",
"vivecraft.options.RENDER_DEBUG_PLAYER_AXES": "Remote Device Axes"
}"#)];

/// Registers the settings' file, their page and their names. Before the game starts.
pub(crate) fn register() -> Result<(), horizon_hooks::TooLate> {
    horizon_hooks::settings::register(&STORE)?;
    horizon_hooks::pages::register(&PAGE)?;
    horizon_hooks::lang::register(LANG)?;
    push(vr_settings());
    Ok(())
}
