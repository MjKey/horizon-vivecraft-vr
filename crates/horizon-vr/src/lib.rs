//! VR through OpenXR (SteamVR, Meta Link), turned on with `HORIZON_VR=1`.
//!
//! Stage 1, a screen in VR: the game's picture on a floating screen 2 m ahead. The renderer opens
//! its Vulkan device here ([`open_device`]) with the extensions the runtime asks for, on the GPU it
//! drives the headset with ([`pick_adapter`]); [`mirror`] then hands each frame to the headset.
//! Anything that fails turns VR off with one line in the log, and the game runs as usual.
use ash::vk::{self, Handle};
use openxr as xr;
use std::ffi::{CStr, CString};
use std::sync::Mutex;
use wgpu::hal::api::Vulkan;

/// Whether the player asked for VR (`HORIZON_VR=1`).
pub fn requested() -> bool {
    std::env::var_os("HORIZON_VR").is_some_and(|v| v != "0" && v != "sim")
}

/// `HORIZON_VR=sim`: no headset; [`simulate`] makes a standing player whose head and hands
/// follow the desktop look, so the game's Vivecraft port runs on the normal window for testing.
pub fn sim_mode() -> bool {
    std::env::var_os("HORIZON_VR").is_some_and(|v| v == "sim")
}

/// The simulated head and hands from the desktop look (`yaw`, `pitch` in Minecraft degrees) and
/// the room's rotation (radians): both controllers point where the player looks.
pub fn simulate(yaw: f32, pitch: f32, room_rotation: f32) {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    let world = glam::Vec3::new(-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos());
    let d = glam::Quat::from_rotation_y(-room_rotation) * world;
    let turn = glam::Quat::from_rotation_y((-d.x).atan2(-d.z));
    let look = turn * glam::Quat::from_rotation_x(d.y.clamp(-1.0, 1.0).asin());
    let pose = |at: glam::Vec3, r: glam::Quat| Pose {
        position: at.to_array(),
        orientation: r.to_array(),
    };
    let head = glam::Vec3::new(0.0, 1.62, 0.0);
    // Each hand held ahead, aiming at the point 3 m along the look (so Vivecraft's look
    // override, which aims the player at the hand's target, keeps the desktop look still).
    let target = head + glam::Vec3::new(d.x, d.y, d.z) * 3.0;
    let facing = |dir: glam::Vec3| {
        let dir = dir.normalize_or_zero();
        glam::Quat::from_rotation_y((-dir.x).atan2(-dir.z))
            * glam::Quat::from_rotation_x(dir.y.clamp(-1.0, 1.0).asin())
    };
    let hand = |side: f32| {
        let at = head + turn * glam::Vec3::new(0.2 * side, -0.3, -0.5);
        Some(pose(at, facing(target - at)))
    };
    let hmd = pose(head, look);
    let eye = |side: f32| pose(head + turn * glam::Vec3::new(0.032 * side, 0.0, 0.0), look);
    *POSES.lock().unwrap_or_else(|e| e.into_inner()) = Some(Poses {
        hmd,
        eyes: [eye(-1.0), eye(1.0)],
        aim: [hand(1.0), hand(-1.0)],
        grip: [hand(1.0), hand(-1.0)],
        turn: 0.0,
    });
}

/// The OpenXR instance and headset, from before the device opens to the session.
struct Start {
    instance: xr::Instance,
    system: xr::SystemId,
    device_extensions: Vec<&'static CStr>,
}

/// The running session and the screen's swapchain.
struct Running {
    session: xr::Session<xr::Vulkan>,
    waiter: xr::FrameWaiter,
    stream: xr::FrameStream<xr::Vulkan>,
    space: xr::Space,
    /// The headset itself (`VIEW`), located in `space` for [`Poses::hmd`].
    view_space: xr::Space,
    instance: xr::Instance,
    active: bool,
    screen: Option<Screen>,
    /// The last frame's picture, copied in the renderer's encoder; handed to the headset at the
    /// next frame, once that encoder is surely submitted.
    held: Option<wgpu::Texture>,
    /// 3D: each eye's swapchain, its copy of the last frame's view, and the frame begun for it.
    eye_chains: [Option<Screen>; 2],
    eye_held: [Option<wgpu::Texture>; 2],
    /// The face effects' pass (`set_eye_tint`).
    tint: Option<Tint>,
    eye_ready: [bool; 2],
    open: Option<Open>,
    /// The runtime's recommended pixels per eye.
    recommended: [u32; 2],
    /// The controllers' actions; `None` when the runtime refused them (the game still shows).
    input: Option<Input>,
    focused: bool,
    /// This frame's headset and hands (main, offhand) in the room.
    hmd: Pose,
    hands: [Option<Pose>; 2],
    /// This frame's room and eye cameras in the world, for drawing the hands in them.
    room: Room,
    eye_cameras: [[f64; 3]; 2],
    /// `TelescopeTracker`: this frame's lens view, the eye looking through it, and the world
    /// point the spyglass points at (where the lens shows in that eye); its last picture.
    lens: Option<(Eye, usize, [f64; 3])>,
    lens_held: Option<wgpu::Texture>,
    /// The desktop window's size, for the third-person mirror's camera picture.
    mirror_size: [u32; 2],
    mixed: Option<Mixed>,
    /// The first-person mirror's picture, drawn in the window's format (`mirror_format`).
    eye_mirror: Option<Mixed>,
    /// Lays [`overlay_eyes`]'s pictures over each eye (premultiplied, multiply).
    eye_overlay: Option<(Mixed, Mixed)>,
    overlay_depth: Option<wgpu::Texture>,
    mirror_format: Option<wgpu::TextureFormat>,
    /// `CameraTracker`: this frame's handheld camera view and its pose in the room, its last
    /// picture, the swapchain of its screen, and a photo being read back.
    camera: Option<(Eye, Pose)>,
    camera_held: Option<wgpu::Texture>,
    camera_chain: Option<Screen>,
    photo: Option<Photo>,
    hand_draw: Option<HandDraw>,
}

/// An OpenXR frame begun for the eyes, ended at the next frame with the views drawn for it.
struct Open {
    time: xr::Time,
    poses: [xr::Posef; 2],
    fovs: [xr::Fovf; 2],
    sizes: [[u32; 2]; 2],
}

struct Screen {
    swapchain: xr::Swapchain<xr::Vulkan>,
    images: Vec<wgpu::Texture>,
    size: [u32; 2],
    /// The source texture format this swapchain was made for.
    format: wgpu::TextureFormat,
    /// To wait for the GPU before the swapchain's images go.
    device: wgpu::Device,
}

impl Drop for Screen {
    // Destroying the swapchain frees its images at once, though the GPU may still be drawing into
    // them from the last frame (a resized window remakes the panel's): the driver then writes
    // freed memory and crashes. So the textures go first, then the GPU finishes, then the
    // swapchain (a field, dropped after this).
    fn drop(&mut self) {
        self.images.clear();
        if let Err(e) = self.device.poll(wgpu::PollType::wait_indefinitely()) {
            log::warn!("Horizon VR: waiting for the GPU before a swapchain goes: {e}");
        }
    }
}

static START: Mutex<Option<Start>> = Mutex::new(None);
/// Set once the connection was tried, so a failure is logged and paid for once.
static TRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

fn off(why: impl std::fmt::Display) {
    log::warn!("Horizon VR: off, {why}");
}

/// Connects to the runtime, once: false, logged, when VR isn't asked for or there is none.
fn start() -> bool {
    let mut start = START.lock().unwrap_or_else(|e| e.into_inner());
    if start.is_some() {
        return true;
    }
    if !requested() || TRIED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    match connect() {
        Ok(s) => {
            *start = Some(s);
            true
        }
        Err(e) => {
            off(e);
            false
        }
    }
}

fn connect() -> Result<Start, String> {
    // Safety: the Khronos loader, beside the game or the system's.
    let entry = unsafe { xr::Entry::load(&()) }
        .map_err(|e| format!("no OpenXR loader (put openxr_loader.dll beside the game): {e}"))?;
    let available = entry
        .enumerate_extensions()
        .map_err(|e| format!("OpenXR runtime (is SteamVR running?): {e}"))?;
    if !available.khr_vulkan_enable {
        return Err("the OpenXR runtime has no Vulkan support (XR_KHR_vulkan_enable)".into());
    }
    let mut enabled = xr::ExtensionSet::default();
    enabled.khr_vulkan_enable = true;
    let instance = entry
        .create_instance(
            &xr::ApplicationInfo {
                application_name: "Horizon VR",
                application_version: 1,
                engine_name: "Horizon",
                engine_version: 1,
                api_version: xr::Version::new(1, 0, 0),
            },
            &enabled,
            &[],
            &(),
        )
        .map_err(|e| format!("OpenXR instance (is SteamVR running?): {e}"))?;
    let props = instance.properties().map_err(|e| e.to_string())?;
    let system = instance
        .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .map_err(|e| format!("no headset (is it connected to SteamVR?): {e}"))?;
    let name = instance
        .system_properties(system)
        .map(|p| p.system_name)
        .unwrap_or_default();
    // The runtime insists on being asked before a session.
    let needs = instance
        .graphics_requirements::<xr::Vulkan>(system)
        .map_err(|e| format!("Vulkan requirements: {e}"))?;
    let instance_extensions = instance
        .vulkan_legacy_instance_extensions(system)
        .unwrap_or_default();
    let device_extensions = instance
        .vulkan_legacy_device_extensions(system)
        .map_err(|e| format!("Vulkan device extensions: {e}"))?;
    log::info!(
        "Horizon VR: {} {} with {name}; Vulkan {}.{} to {}.{}; instance extensions [{instance_extensions}], \
         device extensions [{device_extensions}]",
        props.runtime_name,
        props.runtime_version,
        needs.min_api_version_supported.major(),
        needs.min_api_version_supported.minor(),
        needs.max_api_version_supported.major(),
        needs.max_api_version_supported.minor(),
    );
    Ok(Start {
        instance,
        system,
        device_extensions: device_extensions
            .split_ascii_whitespace()
            .filter_map(|e| CString::new(e).ok())
            .map(|e| &*Box::leak(e.into_boxed_c_str()))
            .collect(),
    })
}

/// The adapter of the GPU the runtime drives the headset with, on `instance` (Vulkan), when it
/// isn't `chosen`; `None` keeps `chosen`.
pub fn pick_adapter(instance: &wgpu::Instance, chosen: &wgpu::Adapter) -> Option<wgpu::Adapter> {
    if !start() {
        return None;
    }
    let start = START.lock().unwrap_or_else(|e| e.into_inner());
    let start = start.as_ref()?;
    // Safety: read-only use of the instance's handles, which outlive this call.
    let hal_instance = unsafe { instance.as_hal::<Vulkan>() }?;
    let raw = hal_instance
        .shared_instance()
        .raw_instance()
        .handle()
        .as_raw();
    let wanted = match unsafe {
        start
            .instance
            .vulkan_graphics_device(start.system, raw as usize as _)
    } {
        Ok(device) => vk::PhysicalDevice::from_raw(device as usize as u64),
        Err(e) => {
            off(format!("no headset GPU: {e}"));
            return None;
        }
    };
    let same =
        unsafe { chosen.as_hal::<Vulkan>() }.is_some_and(|hal| hal.raw_physical_device() == wanted);
    if same {
        return None;
    }
    let exposed = hal_instance.expose_adapter(wanted)?;
    log::info!("Horizon VR: using the headset's GPU, {}", exposed.info.name);
    // Safety: exposed from this instance.
    Some(unsafe { instance.create_adapter_from_hal(exposed) })
}

/// Opens the Vulkan device with the extensions the runtime asks for, and starts the session on
/// it. `None` when VR is off (the renderer opens its device as usual).
pub fn open_device(
    adapter: &wgpu::Adapter,
    desc: &wgpu::DeviceDescriptor<'_>,
) -> Option<Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError>> {
    if !start() {
        return None;
    }
    let start = START.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let open = {
        // Safety: only read and opened from here; dropped before wgpu takes it back.
        let hal = unsafe { adapter.as_hal::<Vulkan>() }?;
        let offered: Vec<CString> = unsafe {
            hal.shared_instance()
                .raw_instance()
                .enumerate_device_extension_properties(hal.raw_physical_device())
        }
        .ok()?
        .iter()
        .filter_map(|e| e.extension_name_as_c_str().ok().map(CStr::to_owned))
        .collect();
        let mut extra = Vec::new();
        for e in &start.device_extensions {
            if offered.iter().any(|o| o.as_c_str() == *e) {
                extra.push(*e);
            } else {
                log::warn!("Horizon VR: the GPU lacks {e:?}, which the runtime asked for");
            }
        }
        // Safety: wgpu's own request_device, with the runtime's extensions added.
        match unsafe {
            hal.open_with_callback(
                desc.required_features,
                &desc.required_limits,
                &desc.memory_hints,
                Some(Box::new(
                    move |args: wgpu::hal::vulkan::CreateDeviceCallbackArgs| {
                        for e in extra {
                            if !args.extensions.contains(&e) {
                                args.extensions.push(e);
                            }
                        }
                    },
                )),
            )
        } {
            Ok(open) => open,
            Err(e) => {
                off(format!("the device with the runtime's extensions: {e}"));
                return None;
            }
        }
    };
    // Safety: opened from this adapter with exactly `desc`'s features.
    let opened = unsafe { adapter.create_device_from_hal(open, desc) };
    if let Ok((device, _)) = &opened {
        match begin_session(start, device) {
            Ok(running) => {
                *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = Some(running);
                log::info!("Horizon VR: session created, put the headset on");
            }
            Err(e) => off(e),
        }
    }
    Some(opened)
}

fn begin_session(start: Start, device: &wgpu::Device) -> Result<Running, String> {
    let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or("not a Vulkan device")?;
    let info = xr::vulkan::SessionCreateInfo {
        instance: hal.shared_instance().raw_instance().handle().as_raw() as usize as _,
        physical_device: hal.raw_physical_device().as_raw() as usize as _,
        device: hal.raw_device().handle().as_raw() as usize as _,
        queue_family_index: hal.queue_family_index(),
        queue_index: hal.queue_index(),
    };
    // Safety: the live device, its instance and queue, which wgpu keeps for the game's lifetime.
    let (session, waiter, stream) = unsafe {
        start
            .instance
            .create_session::<xr::Vulkan>(start.system, &info)
    }
    .map_err(|e| format!("OpenXR session: {e}"))?;
    // Vivecraft's room: the play area with its floor at y = 0 (STAGE), else the seated space
    // lifted to a standing eye height.
    let space =
        match session.create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY) {
            Ok(space) => space,
            Err(_) => session
                .create_reference_space(
                    xr::ReferenceSpaceType::LOCAL,
                    xr::Posef {
                        orientation: xr::Quaternionf::IDENTITY,
                        position: xr::Vector3f {
                            x: 0.0,
                            y: -1.62,
                            z: 0.0,
                        },
                    },
                )
                .map_err(|e| format!("OpenXR space: {e}"))?,
        };
    let view_space = session
        .create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
        .map_err(|e| format!("OpenXR view space: {e}"))?;
    let views = start
        .instance
        .enumerate_view_configuration_views(start.system, xr::ViewConfigurationType::PRIMARY_STEREO)
        .map_err(|e| format!("OpenXR stereo views: {e}"))?;
    let eye = views.first().ok_or("the headset has no stereo view")?;
    let recommended = [
        eye.recommended_image_rect_width,
        eye.recommended_image_rect_height,
    ];
    let input = match Input::new(&start.instance, &session) {
        Ok(input) => Some(input),
        Err(e) => {
            log::warn!("Horizon VR: controllers off, {e}");
            None
        }
    };
    Ok(Running {
        session,
        waiter,
        stream,
        space,
        view_space,
        instance: start.instance.clone(),
        active: false,
        screen: None,
        held: None,
        eye_chains: [None, None],
        eye_held: [None, None],
        tint: None,
        eye_ready: [false, false],
        open: None,
        recommended,
        input,
        focused: false,
        hmd: Pose::default(),
        hands: [None, None],
        room: Room {
            origin: [0.0; 3],
            rotation: 0.0,
            scale: 1.0,
        },
        eye_cameras: [[0.0; 3]; 2],
        lens: None,
        lens_held: None,
        mirror_size: [0, 0],
        mixed: None,
        eye_mirror: None,
        eye_overlay: None,
        overlay_depth: None,
        mirror_format: None,
        camera: None,
        camera_held: None,
        camera_chain: None,
        photo: None,
        hand_draw: None,
    })
}

/// Hands the game's finished picture (`frame`, drawn by `encoder`, not yet submitted) to the
/// headset, one frame late. Does nothing while VR is off.
pub fn mirror(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    frame: &wgpu::Texture,
) {
    let mut lock = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(run) = lock.as_mut() else {
        return;
    };
    if !screen_mode() {
        // 3D: the game's picture is the menu panel's, while a menu is open.
        if (gui_open() || hud_shown()) && run.active {
            run.keep_frame(device, encoder, frame);
        }
        run.mirror_size = [frame.width(), frame.height()];
        run.mirror_format = Some(frame.format());
        if mirror_mixed() {
            if run.camera.is_some()
                && let Some(out) = run.mixed.as_ref().and_then(|m| m.out.as_ref())
                && out.size() == frame.size()
                && out.format() == frame.format()
                && frame.usage().contains(wgpu::TextureUsages::COPY_DST)
            {
                encoder.copy_texture_to_texture(out.as_image_copy(), frame.as_image_copy(), frame.size());
            }
            return;
        }
        // First person: the window shows the left eye (`eye_drawn`), in the window's format only.
        if !mirror_camera()
            && let Some(out) = run.eye_mirror.as_ref().and_then(|m| m.out.as_ref())
            && out.size() == frame.size()
            && out.format() == frame.format()
            && frame.usage().contains(wgpu::TextureUsages::COPY_DST)
        {
            encoder.copy_texture_to_texture(out.as_image_copy(), frame.as_image_copy(), frame.size());
        }
        // `MirrorMode.THIRD_PERSON`: the window shows the handheld camera's picture (sized to it).
        if mirror_camera()
            && run.camera.is_some()
            && let Some(cam) = run.camera_held.as_ref()
            && cam.size() == frame.size()
            && cam.format() == frame.format()
            && frame.usage().contains(wgpu::TextureUsages::COPY_DST)
        {
            encoder.copy_texture_to_texture(cam.as_image_copy(), frame.as_image_copy(), frame.size());
        }
        return;
    }
    if let Err(e) = run.tick(device, queue, encoder, frame) {
        off(e);
        *lock = None;
    }
}

impl Running {
    fn tick(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
    ) -> Result<(), String> {
        self.poll()?;
        if !self.active {
            return Ok(());
        }
        if let Some(held) = self.held.take() {
            self.present(device, queue, &held)?;
        }
        // This frame's picture, for the next one.
        let size = frame.size();
        let held = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon VR held frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: frame.format(),
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        encoder.copy_texture_to_texture(frame.as_image_copy(), held.as_image_copy(), size);
        self.held = Some(held);
        Ok(())
    }

    /// The runtime's events: begins and ends the session as it asks.
    fn poll(&mut self) -> Result<(), String> {
        let mut events = xr::EventDataBuffer::new();
        while let Some(event) = self
            .instance
            .poll_event(&mut events)
            .map_err(|e| e.to_string())?
        {
            use xr::Event::*;
            match event {
                SessionStateChanged(e) => {
                    log::info!("Horizon VR: session {:?}", e.state());
                    self.focused = e.state() == xr::SessionState::FOCUSED;
                    match e.state() {
                        xr::SessionState::READY => {
                            self.session
                                .begin(xr::ViewConfigurationType::PRIMARY_STEREO)
                                .map_err(|e| e.to_string())?;
                            self.active = true;
                        }
                        xr::SessionState::STOPPING => {
                            // A frame can still be open when the runtime asks us to stop.  It
                            // must be ended before `xrEndSession`; otherwise runtimes reject the
                            // next session (and some keep its swapchains acquired).
                            if let Some(open) = self.open.take() {
                                self.stream
                                    .end(open.time, xr::EnvironmentBlendMode::OPAQUE, &[])
                                    .map_err(|e| e.to_string())?;
                            }
                            self.session.end().map_err(|e| e.to_string())?;
                            self.active = false;
                            // No frames run until the next session: don't leave the last poses and buttons up.
                            *POSES.lock().unwrap_or_else(|e| e.into_inner()) = None;
                            *PAD.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                            return Err("the runtime ended the session".into());
                        }
                        _ => {}
                    }
                }
                InstanceLossPending(_) => return Err("the runtime is shutting down".into()),
                _ => {}
            }
        }
        Ok(())
    }

    /// One OpenXR frame: the held picture onto the screen's swapchain, the screen as a quad.
    fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        held: &wgpu::Texture,
    ) -> Result<(), String> {
        let state = self.waiter.wait().map_err(|e| e.to_string())?;
        self.stream.begin().map_err(|e| e.to_string())?;
        if !state.should_render {
            return self
                .stream
                .end(
                    state.predicted_display_time,
                    xr::EnvironmentBlendMode::OPAQUE,
                    &[],
                )
                .map_err(|e| e.to_string());
        }
        let size = [held.width(), held.height()];
        if self
            .screen
            .as_ref()
            .is_none_or(|s| s.size != size || s.format != held.format())
        {
            self.screen = Some(self.make_screen(device, size, held.format())?);
        }
        let screen = self.screen.as_mut().expect("made above");
        let index = screen
            .swapchain
            .acquire_image()
            .map_err(|e| e.to_string())?;
        screen
            .swapchain
            .wait_image(xr::Duration::INFINITE)
            .map_err(|e| e.to_string())?;
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            held.as_image_copy(),
            screen.images[index as usize].as_image_copy(),
            held.size(),
        );
        queue.submit([encoder.finish()]);
        screen
            .swapchain
            .release_image()
            .map_err(|e| e.to_string())?;
        let width = 2.4;
        let quad = xr::CompositionLayerQuad::new()
            .space(&self.space)
            .eye_visibility(xr::EyeVisibility::BOTH)
            .sub_image(
                xr::SwapchainSubImage::new()
                    .swapchain(&screen.swapchain)
                    .image_rect(xr::Rect2Di {
                        offset: xr::Offset2Di { x: 0, y: 0 },
                        extent: xr::Extent2Di {
                            width: size[0] as i32,
                            height: size[1] as i32,
                        },
                    }),
            )
            .pose(xr::Posef {
                orientation: xr::Quaternionf::IDENTITY,
                position: xr::Vector3f {
                    x: 0.0,
                    y: 0.0,
                    z: -2.0,
                },
            })
            .size(xr::Extent2Df {
                width,
                height: width * size[1] as f32 / size[0] as f32,
            });
        self.stream
            .end(
                state.predicted_display_time,
                xr::EnvironmentBlendMode::OPAQUE,
                &[&quad],
            )
            .map_err(|e| e.to_string())
    }

    fn make_screen(
        &self,
        device: &wgpu::Device,
        size: [u32; 2],
        source: wgpu::TextureFormat,
    ) -> Result<Screen, String> {
        // sRGB, so the runtime decodes the game's gamma-encoded picture.
        let (vk_format, format) = match source {
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => (
                vk::Format::B8G8R8A8_SRGB,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ),
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => (
                vk::Format::R8G8B8A8_SRGB,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ),
            wgpu::TextureFormat::Rgba16Float => (
                vk::Format::R16G16B16A16_SFLOAT,
                wgpu::TextureFormat::Rgba16Float,
            ),
            other => return Err(format!("a {other:?} frame (turn HDR off for VR)")),
        };
        let formats = self
            .session
            .enumerate_swapchain_formats()
            .map_err(|e| e.to_string())?;
        if !formats.contains(&(vk_format.as_raw() as u32)) {
            return Err(format!("the runtime offers no {vk_format:?} swapchain"));
        }
        let swapchain = self
            .session
            .create_swapchain(&xr::SwapchainCreateInfo {
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                    | xr::SwapchainUsageFlags::TRANSFER_DST,
                format: vk_format.as_raw() as u32,
                sample_count: 1,
                width: size[0],
                height: size[1],
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })
            .map_err(|e| format!("OpenXR swapchain: {e}"))?;
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or("not a Vulkan device")?;
        let images = swapchain
            .enumerate_images()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|image| {
                // Safety: the runtime's image, alive as long as the swapchain, which outlives
                // these textures (dropped together with the screen).
                let raw = unsafe {
                    hal.texture_from_raw(
                        vk::Image::from_raw(image),
                        &wgpu::hal::TextureDescriptor {
                            label: Some("Horizon VR screen"),
                            size: extent,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage: wgpu::TextureUses::COPY_DST | wgpu::TextureUses::COLOR_TARGET,
                            memory_flags: wgpu::hal::MemoryFlags::empty(),
                            view_formats: vec![],
                        },
                        None,
                        wgpu::hal::vulkan::TextureMemory::External,
                    )
                };
                unsafe {
                    device.create_texture_from_hal::<Vulkan>(
                        raw,
                        &wgpu::TextureDescriptor {
                            label: Some("Horizon VR screen"),
                            size: extent,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage: wgpu::TextureUsages::COPY_DST
                                | wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        },
                        wgpu::TextureUses::UNINITIALIZED,
                    )
                }
            })
            .collect();
        log::info!("Horizon VR: screen {}x{} ({format:?})", size[0], size[1]);
        Ok(Screen {
            swapchain,
            images,
            size,
            format: source,
            device: device.clone(),
        })
    }
}

/// Stage 1's floating screen instead of 3D: `HORIZON_VR=screen`.
pub fn screen_mode() -> bool {
    std::env::var("HORIZON_VR").is_ok_and(|v| v.eq_ignore_ascii_case("screen"))
}

/// The world views the renderer draws for the eyes, left then right.
pub const EYE_IDS: [u64; 2] = [u64::MAX - 1, u64::MAX];

/// One eye's camera, in Horizon's world-view terms: degrees as the player's camera takes them
/// (yaw 0 looks south, pitch up is negative), `fov_degrees` vertical.
#[derive(Clone, Copy, Debug)]
pub struct Eye {
    pub camera: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub fov_degrees: f32,
    pub size: [u32; 2],
}

/// Called once as a frame is built, from the player's eye position `player` and body `yaw`
/// (degrees): hands the last frame's eyes to the headset, begins this one and gives the eyes to
/// draw. `None` while VR is off, in screen mode, or the headset shows nothing this frame.
pub fn eyes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    player: [f64; 3],
    yaw: f32,
) -> Option<[Eye; 2]> {
    if screen_mode() {
        return None;
    }
    let mut lock = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let run = lock.as_mut()?;
    match run.frame_eyes(device, queue, player, yaw) {
        Ok(eyes) => eyes,
        Err(e) => {
            off(e);
            *lock = None;
            None
        }
    }
}

/// The world view of the handheld camera (`CameraTracker`).
pub const CAMERA_ID: u64 = u64::MAX - 3;
/// The camera picture's size (`handCameraResScale` 1 of 1280x720, three quarters).
const CAMERA_SIZE: [u32; 2] = [960, 540];
/// The camera's screen on its back, meters across.
const CAMERA_SCREEN: f32 = 0.2;

/// The handheld camera in the world (position, rotation; its -Z looks), the game tells.
static CAMERA_POSE: Mutex<Option<([f64; 3], glam::Quat)>> = Mutex::new(None);
static PHOTO_PATH: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

pub fn set_camera(pose: Option<([f64; 3], glam::Quat)>) {
    *CAMERA_POSE.lock().unwrap_or_else(|e| e.into_inner()) = pose;
}

/// Takes a photo from the handheld camera at the next frame, saved as a PNG at `path`.
pub fn take_photo(path: std::path::PathBuf) {
    *PHOTO_PATH.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// This frame's camera view while the handheld camera is out.
pub fn camera() -> Option<Eye> {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()?
        .camera
        .map(|c| c.0)
}

/// The camera's world pose in `room` (`Room::to_world`'s inverse) and its view.
fn camera_view(room: Room, size: [u32; 2]) -> Option<(Eye, Pose)> {
    let (pos, rot) = (*CAMERA_POSE.lock().unwrap_or_else(|e| e.into_inner()))?;
    let inv = room.turn().inverse();
    let d = glam::Vec3::new(
        (pos[0] - room.origin[0]) as f32,
        (pos[1] - room.origin[1]) as f32,
        (pos[2] - room.origin[2]) as f32,
    );
    let p = inv * d / room.scale.max(1e-3);
    let q = (inv * rot).normalize();
    let pose = Pose {
        position: p.to_array(),
        orientation: q.to_array(),
    };
    let xr_pose = xr::Posef {
        orientation: xr::Quaternionf {
            x: q.x,
            y: q.y,
            z: q.z,
            w: q.w,
        },
        position: xr::Vector3f {
            x: p.x,
            y: p.y,
            z: p.z,
        },
    };
    let (camera, yaw, pitch, roll) = eye_camera(room, xr_pose);
    Some((
        Eye {
            camera,
            yaw,
            pitch,
            roll,
            fov_degrees: f32::from_bits(CAMERA_FOV_BITS.load(std::sync::atomic::Ordering::Relaxed)),
            size,
        },
        pose,
    ))
}

/// A camera picture copied to a buffer, saved once the copy's commands ran.
struct Photo {
    buffer: wgpu::Buffer,
    size: [u32; 2],
    row: u32,
    format: wgpu::TextureFormat,
    path: std::path::PathBuf,
}

impl Photo {
    fn save(self, device: &wgpu::Device) {
        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if let Err(e) = device.poll(wgpu::PollType::wait_indefinitely()) {
            log::warn!("Horizon VR: photo not read back: {e}");
            return;
        }
        let data = match slice.get_mapped_range() {
            Ok(d) => d,
            Err(e) => {
                log::warn!("Horizon VR: photo not mapped: {e:?}");
                return;
            }
        };
        let bgra = matches!(
            self.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut rgba = Vec::with_capacity((self.size[0] * self.size[1] * 4) as usize);
        for y in 0..self.size[1] {
            let line = &data[(y * self.row) as usize..][..(self.size[0] * 4) as usize];
            for px in line.chunks_exact(4) {
                if bgra {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                } else {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        drop(data);
        let result = (|| -> Result<(), String> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let file = std::fs::File::create(&self.path).map_err(|e| e.to_string())?;
            let mut encoder =
                png::Encoder::new(std::io::BufWriter::new(file), self.size[0], self.size[1]);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            writer.write_image_data(&rgba).map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => log::info!("Horizon VR: photo saved to {}", self.path.display()),
            Err(e) => log::warn!("Horizon VR: photo not saved: {e}"),
        }
    }
}

/// The world view of the spyglass's zoom (`TelescopeTracker`), drawn before the eyes.
pub const LENS_ID: u64 = u64::MAX - 2;

static TELESCOPE: Mutex<Option<usize>> = Mutex::new(None);

/// The hand (0 main, 1 offhand) holding a spyglass, the game tells each tick.
pub fn set_telescope(hand: Option<usize>) {
    *TELESCOPE.lock().unwrap_or_else(|e| e.into_inner()) = hand;
}

/// This frame's lens view while a spyglass is held up to an eye (`isViewing`).
pub fn lens() -> Option<Eye> {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()?
        .lens
        .map(|l| l.0)
}

/// The spyglass's zoom: vanilla's spyglass FOV (`0.1` of 70°).
const LENS_FOV: f32 = 7.0;
/// The lens's share of the eye's picture height while looking through it.
const LENS_SHARE: f32 = 0.35;

impl Running {
    /// The camera's picture onto its screen's swapchain; the screen's pose (on the camera's back,
    /// facing behind it) and size in the room.
    fn camera_screen(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Option<(Pose, [f32; 2])>, String> {
        let (Some((_, pose)), Some(held)) = (self.camera, self.camera_held.as_ref()) else {
            return Ok(None);
        };
        let size = [held.width(), held.height()];
        if self
            .camera_chain
            .as_ref()
            .is_none_or(|c| c.size != size || c.format != held.format())
        {
            self.camera_chain = Some(self.make_screen(device, size, held.format())?);
        }
        let held = self.camera_held.as_ref().expect("checked");
        let chain = self.camera_chain.as_mut().expect("made above");
        let index = chain.swapchain.acquire_image().map_err(|e| e.to_string())?;
        chain
            .swapchain
            .wait_image(xr::Duration::INFINITE)
            .map_err(|e| e.to_string())?;
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            held.as_image_copy(),
            chain.images[index as usize].as_image_copy(),
            held.size(),
        );
        queue.submit([encoder.finish()]);
        chain.swapchain.release_image().map_err(|e| e.to_string())?;
        let rot = pose.rotation();
        let back = pose.translation() + rot * glam::Vec3::new(0.0, 0.0, 0.02);
        let screen = Pose {
            position: back.to_array(),
            orientation: pose.orientation,
        };
        let w = CAMERA_SCREEN;
        Ok(Some((
            screen,
            [w, w * size[1] as f32 / size[0].max(1) as f32],
        )))
    }

    /// `TelescopeTracker.isViewing`: the spyglass's eyepiece (the controller's aim origin)
    /// within 12 cm of an eye, pointing the way that eye looks.
    fn lens_view(
        &self,
        room: Room,
        views: &[xr::View],
        eye_height: u32,
    ) -> Option<(Eye, usize, [f64; 3])> {
        let hand = (*TELESCOPE.lock().unwrap_or_else(|e| e.into_inner()))?;
        let aim = POSES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .and_then(|p| p.aim[hand])?;
        let eye = views.iter().take(2).position(|v| {
            let look = quat(v.pose.orientation) * glam::Vec3::NEG_Z;
            vec3(v.pose.position).distance(aim.translation()) < 0.12
                && look.dot(aim.direction()) > 0.8
        })?;
        let pose = xr::Posef {
            orientation: xr::Quaternionf {
                x: aim.orientation[0],
                y: aim.orientation[1],
                z: aim.orientation[2],
                w: aim.orientation[3],
            },
            position: xr::Vector3f {
                x: aim.position[0],
                y: aim.position[1],
                z: aim.position[2],
            },
        };
        let (camera, yaw, pitch, roll) = eye_camera(room, pose);
        let side = ((eye_height as f32 * LENS_SHARE) as u32).clamp(16, 2048);
        let target = room.to_world(aim.translation() + aim.direction() * 10.0);
        Some((
            Eye {
                camera,
                yaw,
                pitch,
                roll,
                fov_degrees: LENS_FOV,
                size: [side, side],
            },
            eye,
            target,
        ))
    }
}

/// A world view was drawn into `color` (its reversed depth in `depth`, `view_proj` the world
/// relative to `camera` to its clip space) by `encoder`: draws the hands into an eye's picture
/// and keeps it for the headset.
#[allow(clippy::too_many_arguments)]
static HEAD_IN_BLOCK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The headset is inside an opaque block: the eyes show black (`isInsideOpaqueBlock`).
pub fn set_head_in_block(inside: bool) {
    HEAD_IN_BLOCK.store(inside, std::sync::atomic::Ordering::Relaxed);
}

pub fn eye_drawn(
    id: u64,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    color: &wgpu::Texture,
    depth: &wgpu::TextureView,
    view_proj: glam::Mat4,
) {
    let mut lock = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(run) = lock.as_mut() else {
        return;
    };
    if id == CAMERA_ID {
        if run
            .camera_held
            .as_ref()
            .is_none_or(|h| h.size() != color.size() || h.format() != color.format())
        {
            run.camera_held = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon VR camera"),
                size: color.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: color.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let held = run.camera_held.as_ref().expect("made above");
        encoder.copy_texture_to_texture(color.as_image_copy(), held.as_image_copy(), color.size());
        let path = PHOTO_PATH.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(path) = path {
            // shortcut: 8-bit colour frames only (HDR's float frame isn't saved), as VR wants HDR off.
            if color.format().block_copy_size(None) == Some(4) {
                let size = [color.width(), color.height()];
                let row = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Horizon VR photo"),
                    size: u64::from(row * size[1]),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                encoder.copy_texture_to_buffer(
                    color.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(row),
                            rows_per_image: Some(size[1]),
                        },
                    },
                    color.size(),
                );
                run.photo = Some(Photo {
                    buffer,
                    size,
                    row,
                    format: color.format(),
                    path,
                });
            } else {
                log::warn!(
                    "Horizon VR: photo needs HDR off (frame {:?})",
                    color.format()
                );
            }
        }
        // `MIXED_REALITY`: the window's split picture, keyed at the player's depth.
        if mirror_mixed()
            && let Some((eye, _)) = run.camera
            && run.mirror_size[0] >= 32
            && run.mirror_size[1] >= 16
        {
            let head = run.room.to_world(run.hmd.translation());
            let p = glam::Vec3::new(
                (head[0] - eye.camera[0]) as f32,
                (head[1] - eye.camera[1]) as f32,
                (head[2] - eye.camera[2]) as f32,
            );
            let clip = view_proj * p.extend(1.0);
            // Behind the camera: everything is foreground-free (threshold past the near plane).
            let threshold = if clip.w > 0.0 { clip.z / clip.w } else { 2.0 };
            let format = color.format();
            if run.mixed.as_ref().is_none_or(|m| m.format != format) {
                run.mixed = Some(Mixed::new(device, format, None));
            }
            let size = run.mirror_size;
            run.mixed
                .as_mut()
                .expect("made above")
                .draw(device, queue, encoder, color, depth, threshold, size, None);
        }
        return;
    }
    if id == LENS_ID {
        if run
            .lens_held
            .as_ref()
            .is_none_or(|h| h.size() != color.size() || h.format() != color.format())
        {
            run.lens_held = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon VR lens"),
                size: color.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: color.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let held = run.lens_held.as_ref().expect("made above");
        encoder.copy_texture_to_texture(color.as_image_copy(), held.as_image_copy(), color.size());
        return;
    }
    let Some(eye) = EYE_IDS.iter().position(|&e| e == id) else {
        return;
    };
    // The hands are the game's own arms and items now (drawn with the world's entities).
    let _ = (queue, depth, view_proj);
    let held = &mut run.eye_held[eye];
    if held
        .as_ref()
        .is_none_or(|h| h.size() != color.size() || h.format() != color.format())
    {
        *held = Some(device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon VR eye"),
            size: color.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: color.format(),
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }));
    }
    let held = held.as_ref().expect("made above");
    encoder.copy_texture_to_texture(color.as_image_copy(), held.as_image_copy(), color.size());
    // `VREffectsHelper`'s head in a block: the eyes see black, not the caves through the walls.
    if HEAD_IN_BLOCK.load(std::sync::atomic::Ordering::Relaxed) {
        let view = held.create_view(&wgpu::TextureViewDescriptor::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon VR head in block"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if eye == 0 {
            run.mirror_eye(device, queue, encoder, depth);
        }
        return;
    }
    if let Some(t) = *EYE_TINT.lock().unwrap_or_else(|e| e.into_inner()) {
        if run.tint.as_ref().is_none_or(|x| x.format != held.format()) {
            run.tint = Some(Tint::new(device, held.format()));
        }
        let held = run.eye_held[eye].as_ref().expect("made above");
        run.tint.as_ref().expect("made above").draw(queue, encoder, held, t);
    }
    let held = run.eye_held[eye].as_ref().expect("made above");
    // The lens's zoom over where the spyglass points in the eye looking through it.
    if let (Some((_, k, target)), Some(lens)) = (run.lens, run.lens_held.as_ref())
        && k == eye
        && lens.format() == held.format()
    {
        let cam = run.eye_cameras[eye];
        let p = glam::Vec3::new(
            (target[0] - cam[0]) as f32,
            (target[1] - cam[1]) as f32,
            (target[2] - cam[2]) as f32,
        );
        let clip = view_proj * p.extend(1.0);
        let [w, h, side] = [held.width(), held.height(), lens.width().min(lens.height())];
        if clip.w > 0.0 && side <= w && side <= h {
            let x = ((clip.x / clip.w + 1.0) / 2.0 * w as f32) as i64 - i64::from(side / 2);
            let y = ((1.0 - clip.y / clip.w) / 2.0 * h as f32) as i64 - i64::from(side / 2);
            let origin = wgpu::Origin3d {
                x: x.clamp(0, i64::from(w - side)) as u32,
                y: y.clamp(0, i64::from(h - side)) as u32,
                z: 0,
            };
            // Round, as Vivecraft's lens: one copy per row, as wide as the circle there.
            let r = side as f32 / 2.0;
            for row in 0..side {
                let dy = row as f32 + 0.5 - r;
                let half = (r * r - dy * dy).max(0.0).sqrt() as u32;
                if half == 0 {
                    continue;
                }
                let x0 = side / 2 - half.min(side / 2);
                let width = (2 * half).min(side - x0);
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        origin: wgpu::Origin3d {
                            x: x0,
                            y: row,
                            z: 0,
                        },
                        ..lens.as_image_copy()
                    },
                    wgpu::TexelCopyTextureInfo {
                        origin: wgpu::Origin3d {
                            x: origin.x + x0,
                            y: origin.y + row,
                            z: 0,
                        },
                        ..held.as_image_copy()
                    },
                    wgpu::Extent3d {
                        width,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
    if eye == 0 {
        run.mirror_eye(device, queue, encoder, depth);
    }
    run.eye_ready[eye] = true;
}

impl Running {
    /// `MirrorMode.FIRST_PERSON` (cropped): the window's picture from the finished left eye.
    fn mirror_eye(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
    ) {
        let (Some(format), Some(held)) = (self.mirror_format, self.eye_held[0].as_ref()) else {
            return;
        };
        let size = self.mirror_size;
        if mirror_camera() || size[0] < 16 || size[1] < 16 {
            return;
        }
        let (ea, wa) = (held.width() as f32 / held.height().max(1) as f32, size[0] as f32 / size[1] as f32);
        let crop = if wa > ea { [1.0, ea / wa] } else { [wa / ea, 1.0] };
        if self.eye_mirror.as_ref().is_none_or(|m| m.format != format) {
            self.eye_mirror = Some(Mixed::new(device, format, None));
        }
        let held = self.eye_held[0].as_ref().expect("checked above");
        self.eye_mirror
            .as_mut()
            .expect("made above")
            .draw(device, queue, encoder, held, depth, 2.0, size, Some(crop));
    }
}

fn quat(o: xr::Quaternionf) -> glam::Quat {
    glam::Quat::from_xyzw(o.x, o.y, o.z, o.w)
}

fn vec3(p: xr::Vector3f) -> glam::Vec3 {
    glam::Vec3::new(p.x, p.y, p.z)
}

/// A tracked device in the room (meters, OpenXR axes: x right, y up, -z ahead), as Vivecraft's
/// `VRDevicePose` holds it before the room's origin, rotation and scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    /// x, y, z, w.
    pub orientation: [f32; 4],
}

impl Pose {
    fn from_xr(p: xr::Posef) -> Self {
        Self {
            position: [p.position.x, p.position.y, p.position.z],
            orientation: [
                p.orientation.x,
                p.orientation.y,
                p.orientation.z,
                p.orientation.w,
            ],
        }
    }
    pub fn rotation(&self) -> glam::Quat {
        let [x, y, z, w] = self.orientation;
        glam::Quat::from_xyzw(x, y, z, w)
    }
    pub fn translation(&self) -> glam::Vec3 {
        glam::Vec3::from_array(self.position)
    }
    /// Where the device points (its -Z), in the room.
    pub fn direction(&self) -> glam::Vec3 {
        self.rotation() * glam::Vec3::NEG_Z
    }
}

/// Everything tracked this frame, in the room: the game's Vivecraft port reads it each tick.
/// Hands are Vivecraft's order: 0 the main (right) hand, 1 the offhand (left).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Poses {
    pub hmd: Pose,
    pub eyes: [Pose; 2],
    /// The controllers' pointing rays (`c0`, `c1`).
    pub aim: [Option<Pose>; 2],
    /// The controllers' grips (`h0`, `h1`).
    pub grip: [Option<Pose>; 2],
    /// The right stick's x, which turns the room (`keyRotateAxis`).
    pub turn: f32,
}

static POSES: Mutex<Option<Poses>> = Mutex::new(None);

/// The last frame's tracking, while the headset shows the game.
pub fn poses() -> Option<Poses> {
    *POSES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Vivecraft's room transform (`VRData.origin`, `rotation_radians`, `worldScale`): a device's
/// world position is `origin + rotateY(rotation) * (scale * room position)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub origin: [f64; 3],
    pub rotation: f32,
    pub scale: f32,
}

impl Room {
    pub fn turn(&self) -> glam::Quat {
        glam::Quat::from_rotation_y(self.rotation)
    }
    /// A room position in the world (`VRDevicePose.getPosition`).
    pub fn to_world(&self, p: glam::Vec3) -> [f64; 3] {
        let v = self.turn() * (p * self.scale);
        [
            self.origin[0] + f64::from(v.x),
            self.origin[1] + f64::from(v.y),
            self.origin[2] + f64::from(v.z),
        ]
    }
    /// A room direction in the world (`VRDevicePose.getDirection`).
    pub fn dir_to_world(&self, d: glam::Vec3) -> glam::Vec3 {
        self.turn() * d
    }
}

struct RoomState {
    from: Room,
    to: Room,
    at: std::time::Instant,
}

static ROOM: Mutex<Option<RoomState>> = Mutex::new(None);

/// The game's room after a tick; frames ease from the last one to it over the tick
/// (`VRPlayer.preRender`'s interpolation).
pub fn set_room(room: Option<Room>) {
    let mut lock = ROOM.lock().unwrap_or_else(|e| e.into_inner());
    *lock = room.map(|to| RoomState {
        from: lock.as_ref().map_or(to, RoomState::now),
        to,
        at: std::time::Instant::now(),
    });
}

impl RoomState {
    fn now(&self) -> Room {
        let t = (self.at.elapsed().as_secs_f64() / 0.05).clamp(0.0, 1.0);
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        Room {
            origin: std::array::from_fn(|i| lerp(self.from.origin[i], self.to.origin[i])),
            // Turns snap, as a 45 degree snap turn eases over one tick in Vivecraft.
            rotation: self.to.rotation,
            scale: lerp(f64::from(self.from.scale), f64::from(self.to.scale)) as f32,
        }
    }
}

/// The room this frame: the game's, else one that stands the headset at the player's eyes
/// facing their yaw (before the game's port has a room, on the first frames).
fn frame_room(player: [f64; 3], yaw: f32, hmd: Pose) -> Room {
    if let Some(room) = ROOM.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return room.now();
    }
    let rotation = std::f32::consts::PI - yaw.to_radians();
    let head = glam::Quat::from_rotation_y(rotation) * hmd.translation();
    Room {
        origin: [
            player[0] - f64::from(head.x),
            player[1] - f64::from(head.y),
            player[2] - f64::from(head.z),
        ],
        rotation,
        scale: 1.0,
    }
}

/// The camera an eye's pose gives in Horizon's world. Horizon's camera
/// (`camera::view_matrix_rolled`) is the rotation Y(180° - yaw), X(-pitch), Z(-roll) of a camera
/// looking down -Z, as an OpenXR view is.
fn eye_camera(room: Room, pose: xr::Posef) -> ([f64; 3], f32, f32, f32) {
    use std::f32::consts::PI;
    let (a, b, c) = (room.turn() * quat(pose.orientation)).to_euler(glam::EulerRot::YXZ);
    (
        room.to_world(vec3(pose.position)),
        (PI - a).to_degrees(),
        -b.to_degrees(),
        -c.to_degrees(),
    )
}

impl Running {
    fn frame_eyes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        player: [f64; 3],
        yaw: f32,
    ) -> Result<Option<[Eye; 2]>, String> {
        self.poll()?;
        if let Some(photo) = self.photo.take() {
            photo.save(device);
        }
        if let Some(open) = self.open.take() {
            self.end_eyes(device, queue, open)?;
        }
        if !self.active {
            return Ok(None);
        }
        let state = self.waiter.wait().map_err(|e| e.to_string())?;
        self.stream.begin().map_err(|e| e.to_string())?;
        self.read_controllers();
        self.locate_devices(state.predicted_display_time);
        self.place_gui();
        let room = frame_room(player, yaw, self.hmd);
        self.room = room;
        let views = if state.should_render {
            self.session
                .locate_views(
                    xr::ViewConfigurationType::PRIMARY_STEREO,
                    state.predicted_display_time,
                    &self.space,
                )
                .map_err(|e| e.to_string())?
                .1
        } else {
            Vec::new()
        };
        if views.len() < 2 {
            self.stream
                .end(
                    state.predicted_display_time,
                    xr::EnvironmentBlendMode::OPAQUE,
                    &[],
                )
                .map_err(|e| e.to_string())?;
            return Ok(None);
        }
        let mut eyes = [None, None];
        let mut fovs = [xr::Fovf::default(); 2];
        let mut sizes = [[0; 2]; 2];
        for (k, view) in views.iter().take(2).enumerate() {
            // Horizon's views are symmetric: the widest half-angles each way, told to the
            // runtime as drawn, so it reprojects them right.
            let f = view.fov;
            let h = f.angle_right.max(-f.angle_left);
            let v = f.angle_up.max(-f.angle_down);
            let height = self.recommended[1].clamp(16, 4096);
            let width = ((height as f32 * h.tan() / v.tan()).round() as u32).clamp(16, 4096);
            fovs[k] = xr::Fovf {
                angle_left: -h,
                angle_right: h,
                angle_up: v,
                angle_down: -v,
            };
            sizes[k] = [width, height];
            let (camera, yaw, pitch, roll) = eye_camera(room, view.pose);
            self.eye_cameras[k] = camera;
            eyes[k] = Some(Eye {
                camera,
                yaw,
                pitch,
                roll,
                fov_degrees: (2.0 * v).to_degrees(),
                size: [width, height],
            });
        }
        self.lens = self.lens_view(room, &views, sizes[0][1]);
        let camera_size = if mirror_mixed() && self.mirror_size[0] >= 32 && self.mirror_size[1] >= 16 {
            [(self.mirror_size[0] / 2).min(4096), self.mirror_size[1].min(4096)]
        } else if mirror_camera() && self.mirror_size[0] >= 16 && self.mirror_size[1] >= 16 {
            self.mirror_size.map(|v| v.min(4096))
        } else {
            CAMERA_SIZE
        };
        self.camera = camera_view(room, camera_size);
        self.eye_ready = [false, false];
        self.open = Some(Open {
            time: state.predicted_display_time,
            poses: [views[0].pose, views[1].pose],
            fovs,
            sizes,
        });
        Ok(Some([eyes[0].expect("set"), eyes[1].expect("set")]))
    }

    /// Ends the frame `open` began: each eye's kept picture onto its swapchain, both as one
    /// projection layer. An eye not drawn (a frame without the world) ends it empty.
    fn end_eyes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        open: Open,
    ) -> Result<(), String> {
        let ready = self.eye_ready == [true, true]
            && self
                .eye_held
                .iter()
                .zip(open.sizes)
                .all(|(h, s)| h.as_ref().is_some_and(|h| [h.width(), h.height()] == s));
        let panel = self.gui_panel_ready() && self.prepare_panel(device)?;
        let camera = self.camera_screen(device, queue)?;
        let camera_quad = camera.and_then(|(pose, [w, h])| {
            let chain = self.camera_chain.as_ref()?;
            let size = chain.size;
            Some(
                xr::CompositionLayerQuad::new()
                    .space(&self.space)
                    .eye_visibility(xr::EyeVisibility::BOTH)
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&chain.swapchain)
                            .image_rect(xr::Rect2Di {
                                offset: xr::Offset2Di { x: 0, y: 0 },
                                extent: xr::Extent2Di {
                                    width: size[0] as i32,
                                    height: size[1] as i32,
                                },
                            }),
                    )
                    .pose(xr::Posef {
                        orientation: xr::Quaternionf {
                            x: pose.orientation[0],
                            y: pose.orientation[1],
                            z: pose.orientation[2],
                            w: pose.orientation[3],
                        },
                        position: xr::Vector3f {
                            x: pose.position[0],
                            y: pose.position[1],
                            z: pose.position[2],
                        },
                    })
                    .size(xr::Extent2Df {
                        width: w,
                        height: h,
                    }),
            )
        });
        if !ready && panel {
            let before: Vec<&xr::CompositionLayerBase<'_, xr::Vulkan>> =
                camera_quad.iter().map(|q| &**q).collect();
            return end_panel(
                &mut self.stream,
                &self.space,
                self.screen.as_mut().expect("prepared"),
                self.held.as_ref().expect("prepared"),
                device,
                queue,
                open.time,
                &before,
            );
        }
        if !ready {
            static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 900 == 0 {
                log::info!(
                    "Horizon VR: eyes not ready (drawn {:?}, kept {:?}, wanted {:?})",
                    self.eye_ready,
                    self.eye_held
                        .each_ref()
                        .map(|h| h.as_ref().map(|h| [h.width(), h.height()])),
                    open.sizes
                );
            }
            return self
                .stream
                .end(open.time, xr::EnvironmentBlendMode::OPAQUE, &[])
                .map_err(|e| e.to_string());
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut indices = [0u32; 2];
        for k in 0..2 {
            let held = self.eye_held[k].as_ref().expect("checked");
            let size = [held.width(), held.height()];
            if self.eye_chains[k]
                .as_ref()
                .is_none_or(|c| c.size != size || c.format != held.format())
            {
                self.eye_chains[k] = Some(self.make_screen(device, size, held.format())?);
            }
            let chain = self.eye_chains[k].as_mut().expect("made above");
            indices[k] = chain.swapchain.acquire_image().map_err(|e| e.to_string())?;
            chain
                .swapchain
                .wait_image(xr::Duration::INFINITE)
                .map_err(|e| e.to_string())?;
            encoder.copy_texture_to_texture(
                held.as_image_copy(),
                chain.images[indices[k] as usize].as_image_copy(),
                held.size(),
            );
        }
        queue.submit([encoder.finish()]);
        for chain in self.eye_chains.iter_mut().flatten() {
            chain.swapchain.release_image().map_err(|e| e.to_string())?;
        }
        let [left, right] = [0, 1].map(|k| {
            let chain = self.eye_chains[k].as_ref().expect("made above");
            xr::CompositionLayerProjectionView::new()
                .pose(open.poses[k])
                .fov(open.fovs[k])
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&chain.swapchain)
                        .image_rect(xr::Rect2Di {
                            offset: xr::Offset2Di { x: 0, y: 0 },
                            extent: xr::Extent2Di {
                                width: open.sizes[k][0] as i32,
                                height: open.sizes[k][1] as i32,
                            },
                        }),
                )
        });
        let views = [left, right];
        let layer = xr::CompositionLayerProjection::new()
            .space(&self.space)
            .views(&views);
        let mut before: Vec<&xr::CompositionLayerBase<'_, xr::Vulkan>> = vec![&layer];
        before.extend(camera_quad.iter().map(|q| &**q));
        if panel {
            return end_panel(
                &mut self.stream,
                &self.space,
                self.screen.as_mut().expect("prepared"),
                self.held.as_ref().expect("prepared"),
                device,
                queue,
                open.time,
                &before,
            );
        }
        self.stream
            .end(open.time, xr::EnvironmentBlendMode::OPAQUE, &before)
            .map_err(|e| e.to_string())
    }
}

/// The Quest controllers as a gamepad, for Horizon's controller support (`docs/controllers.md`):
/// sticks -1..1 with up positive, triggers 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub triggers: [f32; 2],
    /// A, B, X, Y as a pad's South, East, West, North.
    pub south: bool,
    pub east: bool,
    pub west: bool,
    pub north: bool,
    /// The grips, as the bumpers (hotbar left/right).
    pub left_bumper: bool,
    pub right_bumper: bool,
    /// The left controller's menu button, as Start (pause).
    pub start: bool,
    pub left_stick: bool,
    pub right_stick: bool,
}

static PAD: Mutex<Option<Pad>> = Mutex::new(None);
static HOLD_Y: Mutex<HoldY> = Mutex::new(HoldY { since: None, tap_until: None });

/// Y: a tap is North (the inventory), a hold is Start (pause), since the runtime can take the
/// left menu button (SteamVR's dashboard on a Quest).
struct HoldY {
    since: Option<std::time::Instant>,
    /// A tap shows as North until then, long enough for the game to see the press.
    tap_until: Option<std::time::Instant>,
}

impl HoldY {
    const HOLD: std::time::Duration = std::time::Duration::from_millis(500);
    const TAP: std::time::Duration = std::time::Duration::from_millis(100);

    /// This frame's (North, Start) from whether Y is down.
    fn step(&mut self, down: bool, now: std::time::Instant) -> (bool, bool) {
        if down {
            let since = *self.since.get_or_insert(now);
            return (false, now - since >= Self::HOLD);
        }
        if let Some(since) = self.since.take()
            && now - since < Self::HOLD
        {
            self.tap_until = Some(now + Self::TAP);
        }
        (self.tap_until.is_some_and(|t| now < t), false)
    }
}

#[cfg(test)]
mod hold_y_tests {
    use super::HoldY;
    use std::time::{Duration, Instant};

    #[test]
    fn a_tap_is_north_and_a_hold_is_start() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut y = HoldY { since: None, tap_until: None };
        // Tap: nothing while down, North for a moment after the release.
        assert_eq!(y.step(true, ms(0)), (false, false));
        assert_eq!(y.step(false, ms(200)), (true, false));
        assert_eq!(y.step(false, ms(250)), (true, false));
        assert_eq!(y.step(false, ms(400)), (false, false));
        // Hold: Start from half a second while down, no North after.
        assert_eq!(y.step(true, ms(1000)), (false, false));
        assert_eq!(y.step(true, ms(1600)), (false, true));
        assert_eq!(y.step(false, ms(1700)), (false, false));
    }
}

/// The controllers as they were at the last frame, while VR runs with the headset on.
pub fn pad() -> Option<Pad> {
    *PAD.lock().unwrap_or_else(|e| e.into_inner())
}

struct Input {
    set: xr::ActionSet,
    walk: xr::Action<xr::Vector2f>,
    turn: xr::Action<xr::Vector2f>,
    triggers: [xr::Action<f32>; 2],
    grips: [xr::Action<f32>; 2],
    /// A, B, X, Y, menu, left stick click, right stick click.
    buttons: [xr::Action<bool>; 7],
    /// The hands' grips (left, right) and the right hand's pointing ray, as spaces.
    grip_spaces: [xr::Space; 2],
    aim_spaces: [xr::Space; 2],
    _poses: [xr::Action<xr::Posef>; 4],
    /// Vibration for the main (right) and offhand (left) controllers.
    haptics: [xr::Action<xr::Haptic>; 2],
}

impl Input {
    fn new(instance: &xr::Instance, session: &xr::Session<xr::Vulkan>) -> Result<Self, String> {
        let e = |e: xr::sys::Result| e.to_string();
        let set = instance
            .create_action_set("gameplay", "Gameplay", 0)
            .map_err(e)?;
        let walk = set.create_action("walk", "Walk", &[]).map_err(e)?;
        let turn = set.create_action("turn", "Turn", &[]).map_err(e)?;
        let triggers = [
            set.create_action("use", "Use / place", &[]).map_err(e)?,
            set.create_action("attack", "Attack / break", &[])
                .map_err(e)?,
        ];
        let grips = [
            set.create_action("hotbar_left", "Hotbar left", &[])
                .map_err(e)?,
            set.create_action("hotbar_right", "Hotbar right", &[])
                .map_err(e)?,
        ];
        let buttons = [
            set.create_action("jump", "Jump", &[]).map_err(e)?,
            set.create_action("b", "B", &[]).map_err(e)?,
            set.create_action("x", "X", &[]).map_err(e)?,
            set.create_action("inventory", "Inventory", &[])
                .map_err(e)?,
            set.create_action("pause", "Pause", &[]).map_err(e)?,
            set.create_action("sprint", "Sprint", &[]).map_err(e)?,
            set.create_action("sneak", "Sneak", &[]).map_err(e)?,
        ];
        let poses = [
            set.create_action("left_hand", "Left hand", &[])
                .map_err(e)?,
            set.create_action("right_hand", "Right hand", &[])
                .map_err(e)?,
            set.create_action("aim", "Aim", &[]).map_err(e)?,
            set.create_action("left_aim", "Left aim", &[]).map_err(e)?,
        ];
        let haptics = [
            set.create_action("haptic_main", "Main hand vibration", &[])
                .map_err(e)?,
            set.create_action("haptic_off", "Offhand vibration", &[])
                .map_err(e)?,
        ];
        let path = |p: &str| instance.string_to_path(p).map_err(e);
        let profile = path("/interaction_profiles/oculus/touch_controller")?;
        let mut bindings = vec![
            xr::Binding::new(&walk, path("/user/hand/left/input/thumbstick")?),
            xr::Binding::new(&turn, path("/user/hand/right/input/thumbstick")?),
            xr::Binding::new(&triggers[0], path("/user/hand/left/input/trigger/value")?),
            xr::Binding::new(&triggers[1], path("/user/hand/right/input/trigger/value")?),
            xr::Binding::new(&grips[0], path("/user/hand/left/input/squeeze/value")?),
            xr::Binding::new(&grips[1], path("/user/hand/right/input/squeeze/value")?),
            xr::Binding::new(&poses[0], path("/user/hand/left/input/grip/pose")?),
            xr::Binding::new(&poses[1], path("/user/hand/right/input/grip/pose")?),
            xr::Binding::new(&poses[2], path("/user/hand/right/input/aim/pose")?),
            xr::Binding::new(&poses[3], path("/user/hand/left/input/aim/pose")?),
            xr::Binding::new(&haptics[0], path("/user/hand/right/output/haptic")?),
            xr::Binding::new(&haptics[1], path("/user/hand/left/output/haptic")?),
        ];
        for (action, p) in buttons.iter().zip([
            "/user/hand/right/input/a/click",
            "/user/hand/right/input/b/click",
            "/user/hand/left/input/x/click",
            "/user/hand/left/input/y/click",
            "/user/hand/left/input/menu/click",
            "/user/hand/left/input/thumbstick/click",
            "/user/hand/right/input/thumbstick/click",
        ]) {
            bindings.push(xr::Binding::new(action, path(p)?));
        }
        instance
            .suggest_interaction_profile_bindings(profile, &bindings)
            .map_err(|e| format!("Touch controller bindings: {e}"))?;
        session.attach_action_sets(&[&set]).map_err(e)?;
        let space = |a: &xr::Action<xr::Posef>| {
            a.create_space(session, xr::Path::NULL, xr::Posef::IDENTITY)
                .map_err(e)
        };
        let grip_spaces = [space(&poses[0])?, space(&poses[1])?];
        let aim_spaces = [space(&poses[3])?, space(&poses[2])?];
        log::info!("Horizon VR: controllers ready (Touch bindings)");
        Ok(Self {
            grip_spaces,
            aim_spaces,
            haptics,
            _poses: poses,
            set,
            walk,
            turn,
            triggers,
            grips,
            buttons,
        })
    }
}

impl Running {
    /// Reads the controllers into [`pad`]; none while the headset isn't focused on the game.
    fn read_controllers(&mut self) {
        let pad = self
            .input
            .as_ref()
            .filter(|_| self.focused)
            .and_then(|input| {
                self.session
                    .sync_actions(&[xr::ActiveActionSet::new(&input.set)])
                    .ok()?;
                let session = &self.session;
                let v2 = |a: &xr::Action<xr::Vector2f>| {
                    a.state(session, xr::Path::NULL)
                        .map(|s| [s.current_state.x, s.current_state.y])
                        .unwrap_or_default()
                };
                let f = |a: &xr::Action<f32>| {
                    a.state(session, xr::Path::NULL)
                        .map(|s| s.current_state)
                        .unwrap_or_default()
                };
                let b = |k: usize| {
                    input.buttons[k]
                        .state(session, xr::Path::NULL)
                        .is_ok_and(|s| s.current_state)
                };
                // SteamVR on a Quest keeps the left menu button for its own menu: holding Y pauses.
                let y = HOLD_Y
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .step(b(3), std::time::Instant::now());
                Some(Pad {
                    left: v2(&input.walk),
                    // The right stick snap-turns the body (`Running::snap_turn`), the right hand aims.
                    right: v2(&input.turn),
                    triggers: [f(&input.triggers[0]), f(&input.triggers[1])],
                    south: b(0),
                    east: b(1),
                    west: b(2),
                    north: y.0,
                    left_bumper: f(&input.grips[0]) > 0.6,
                    right_bumper: f(&input.grips[1]) > 0.6,
                    start: b(4) || y.1,
                    left_stick: b(5),
                    right_stick: b(6),
                })
            });
        if pad.is_none() {
            // Unfocused: forget a held Y, or coming back would pause at once.
            *HOLD_Y.lock().unwrap_or_else(|e| e.into_inner()) = HoldY { since: None, tap_until: None };
        }
        // `reverseHands`: the left controller is the main one; every binding moves with its hand.
        let pad = pad.map(|p| {
            if !reversed() {
                return p;
            }
            Pad {
                left: p.right,
                right: p.left,
                triggers: [p.triggers[1], p.triggers[0]],
                south: p.west,
                east: p.north,
                west: p.south,
                north: p.east,
                left_bumper: p.right_bumper,
                right_bumper: p.left_bumper,
                start: p.start,
                left_stick: p.right_stick,
                right_stick: p.left_stick,
            }
        });
        *PAD.lock().unwrap_or_else(|e| e.into_inner()) = pad;
        let pulses = std::mem::take(&mut *HAPTICS.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(input) = self.input.as_ref().filter(|_| self.focused) {
            for (c, micros) in pulses {
                let vibration = xr::HapticVibration::new()
                    .amplitude(1.0)
                    .frequency(xr::FREQUENCY_UNSPECIFIED)
                    .duration(xr::Duration::from_nanos(i64::from(micros) * 1000));
                let c = if reversed() { 1 - c.min(1) } else { c.min(1) };
                let _ = input.haptics[c].apply_feedback(
                    &self.session,
                    xr::Path::NULL,
                    &vibration,
                );
            }
        }
    }
}

impl Running {
    /// The headset, eyes and controllers at the frame's display time, published as [`poses`].
    fn locate_devices(&mut self, time: xr::Time) {
        let tracked = |l: xr::SpaceLocation| {
            l.location_flags
                .contains(
                    xr::SpaceLocationFlags::POSITION_VALID
                        | xr::SpaceLocationFlags::ORIENTATION_VALID,
                )
                .then_some(Pose::from_xr(l.pose))
        };
        if let Ok(l) = self.view_space.locate(&self.space, time)
            && let Some(hmd) = tracked(l)
        {
            self.hmd = hmd;
        }
        let eyes = self
            .session
            .locate_views(xr::ViewConfigurationType::PRIMARY_STEREO, time, &self.space)
            .ok()
            .filter(|(_, v)| v.len() >= 2)
            .map(|(_, v)| [Pose::from_xr(v[0].pose), Pose::from_xr(v[1].pose)])
            .unwrap_or([self.hmd; 2]);
        let (aim, grip, turn) = match self.input.as_ref() {
            Some(input) => {
                let at = |space: &xr::Space| space.locate(&self.space, time).ok().and_then(tracked);
                let turn = if self.focused {
                    (if reversed() { &input.walk } else { &input.turn })
                        .state(&self.session, xr::Path::NULL)
                        .map_or(0.0, |s| s.current_state.x)
                } else {
                    0.0
                };
                // Vivecraft's order: main (right, or left with `reverseHands`) hand first.
                let [m, o] = if reversed() { [0, 1] } else { [1, 0] };
                (
                    [at(&input.aim_spaces[m]), at(&input.aim_spaces[o])],
                    [at(&input.grip_spaces[m]), at(&input.grip_spaces[o])],
                    turn,
                )
            }
            None => ([None; 2], [None; 2], 0.0),
        };
        self.hands = grip;
        let poses = Poses {
            hmd: self.hmd,
            eyes,
            aim,
            grip,
            turn,
        };
        *POSES.lock().unwrap_or_else(|e| e.into_inner()) = self.focused.then_some(poses);
    }

    /// Draws a block for each tracked hand into an eye's picture.
    #[allow(clippy::too_many_arguments)]
    fn draw_hands(
        &mut self,
        eye: usize,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::Texture,
        depth: &wgpu::TextureView,
        view_proj: glam::Mat4,
    ) {
        if self.hands.iter().all(Option::is_none) {
            return;
        }
        let format = color.format();
        if self.hand_draw.as_ref().is_none_or(|d| d.format != format) {
            self.hand_draw = Some(HandDraw::new(device, format));
        }
        let draw = self.hand_draw.as_ref().expect("made above");
        let room = self.room;
        let camera = self.eye_cameras[eye];
        let mut slots = Vec::new();
        for (k, hand) in self.hands.iter().enumerate() {
            let Some(pose) = hand else {
                continue;
            };
            let at = room.to_world(pose.translation());
            // The hand relative to this eye's camera, as `view_proj` takes the world.
            let offset = glam::Vec3::new(
                (at[0] - camera[0]) as f32,
                (at[1] - camera[1]) as f32,
                (at[2] - camera[2]) as f32,
            );
            // A fist about the grip: 8 cm wide, 9 cm tall, 14 cm long, centred a little ahead.
            let model =
                glam::Mat4::from_rotation_translation(room.turn() * pose.rotation(), offset)
                    * glam::Mat4::from_scale(glam::Vec3::splat(room.scale))
                    * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.01, 0.02))
                    * glam::Mat4::from_scale(glam::Vec3::new(0.08, 0.09, 0.14));
            let slot = eye * 2 + k;
            let uniform = HandUniform {
                mvp: (view_proj * model).to_cols_array(),
                color: [0.78, 0.6, 0.45, 1.0],
            };
            queue.write_buffer(&draw.uniforms, (slot * HAND_SLOT) as u64, &uniform.bytes());
            slots.push(slot);
        }
        let view = color.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon VR hands"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&draw.pipeline);
        pass.set_vertex_buffer(0, draw.cube.slice(..));
        for slot in slots {
            pass.set_bind_group(0, &draw.bind_group, &[(slot * HAND_SLOT) as u32]);
            pass.draw(0..36, 0..1);
        }
    }
}

/// One hand's draw: its transform and colour, at a 256-byte slot (eye * 2 + hand).
const HAND_SLOT: usize = 256;

struct HandUniform {
    mvp: [f32; 16],
    color: [f32; 4],
}

impl HandUniform {
    fn bytes(&self) -> Vec<u8> {
        self.mvp
            .iter()
            .chain(&self.color)
            .flat_map(|f| f.to_le_bytes())
            .collect()
    }
}

/// The hands' pipeline: unit cubes, shaded by face, depth-tested against the eye's world.
struct HandDraw {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    cube: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

const HAND_SHADER: &str = r"
struct Hand { mvp: mat4x4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> hand: Hand;
struct Out { @builtin(position) pos: vec4<f32>, @location(0) shade: f32 };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) shade: f32) -> Out {
    var out: Out;
    out.pos = hand.mvp * vec4<f32>(pos, 1.0);
    out.shade = shade;
    return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(hand.color.rgb * in.shade, 1.0);
}
";

impl HandDraw {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon VR hands"),
            source: wgpu::ShaderSource::Wgsl(HAND_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon VR hands"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon VR hands"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Horizon VR hands"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            // Horizon's depth is reversed: nearer is greater.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let cube = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon VR hand cube"),
            size: (36 * 16) as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: true,
        });
        cube.slice(..)
            .get_mapped_range_mut()
            .expect("mapped at creation")
            .copy_from_slice(&cube_vertices());
        cube.unmap();
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon VR hands"),
            size: (4 * HAND_SLOT) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon VR hands"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: std::num::NonZeroU64::new(80),
                }),
            }],
        });
        Self {
            format,
            pipeline,
            cube,
            uniforms,
            bind_group,
        }
    }
}

/// A cube from -0.5 to 0.5, both windings drawn (no culling): position and a shade per face.
fn cube_vertices() -> Vec<u8> {
    let faces: [([f32; 3], [f32; 3], [f32; 3], f32); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0.8),
        ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0.8),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 1.0),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 0.5),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.65),
        ([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.65),
    ];
    let mut out = Vec::new();
    for (n, u, v, shade) in faces {
        let corner =
            |a: f32, b: f32| [0, 1, 2].map(|i| 0.5 * n[i] + 0.5 * a * u[i] + 0.5 * b * v[i]);
        for (a, b) in [
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
        ] {
            for f in corner(a, b).iter().chain(std::iter::once(&shade)) {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
    }
    out
}

static HAPTICS: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());

/// `MCVR.triggerHapticPulse(controller, durationMicros)`: 0 the main hand, 1 the offhand.
pub fn haptic(controller: usize, micros: u32) {
    let mut queue = HAPTICS.lock().unwrap_or_else(|e| e.into_inner());
    if queue.len() < 16 {
        queue.push((controller, micros.max(2000)));
    }
}

/// The room as this frame draws it (eased between ticks), for placing things at the hands.
pub fn render_room() -> Option<Room> {
    ROOM.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(RoomState::now)
}

/// Vivecraft's `GuiHandler`: menus on a panel in the room, pointed at with the main controller.
struct Gui {
    open: bool,
    /// The panel's centre and turn in the room, placed in front of the head as a menu opens.
    panel: Option<(glam::Vec3, glam::Quat)>,
    /// Where the main controller's ray meets the panel, 0..1 across and down.
    pointer: Option<[f32; 2]>,
    /// The game is in a world with no menu: the HUD shows on the offhand's wrist.
    hud: bool,
    /// The panel's size (meters) when placed.
    size: [f32; 2],
}

static GUI: Mutex<Gui> = Mutex::new(Gui {
    open: false,
    panel: None,
    pointer: None,
    hud: false,
    size: [0.0; 2],
});

/// The HUD on the wrist this frame (`HUDLock.WRIST`, `HUD_POPUP`): its centre, rotation (in the
/// room, x right and y up across the picture) and size in meters.
pub fn wrist_hud() -> Option<(glam::Vec3, glam::Quat, [f32; 2])> {
    let g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    (g.hud && !g.open && !HUD_HEAD.load(std::sync::atomic::Ordering::Relaxed))
        .then_some(g.panel.map(|(c, r)| (c, r, g.size)))
        .flatten()
}

/// `VRSettings.vrHudLockMode = WRIST`: the HUD panel's width on the wrist (meters).
const HUD_WIDTH: f32 = 0.4;
static MIRROR_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// `displayMirrorMode`: 0 first person, 1 `THIRD_PERSON` (the handheld camera), 2
/// `MIXED_REALITY` (the camera, and beside it what is nearer to it than the player, on green).
pub fn set_mirror_mode(mode: u8) {
    MIRROR_MODE.store(mode.min(2), std::sync::atomic::Ordering::Relaxed);
}

fn mirror_camera() -> bool {
    MIRROR_MODE.load(std::sync::atomic::Ordering::Relaxed) != 0
}

fn mirror_mixed() -> bool {
    MIRROR_MODE.load(std::sync::atomic::Ordering::Relaxed) == 2
}

/// `MIXED_REALITY`'s picture: the camera view left, its foreground keyed on green right.

/// Vivecraft's face effects in the eyes (`VREffectsHelper`: pumpkin, portal, freeze, low health,
/// fire): a colour over the eye's picture, `vignette` 1 keeps it to the edges.
static EYE_TINT: Mutex<Option<([f32; 4], f32)>> = Mutex::new(None);

/// The eyes' picture size and vertical field of view (degrees) this frame, for the screen
/// effects drawn over them ([`overlay_eyes`]).
pub fn eye_screen() -> Option<([u32; 2], f32)> {
    let lock = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let open = lock.as_ref()?.open.as_ref()?;
    let f = open.fovs[0];
    Some((open.sizes[0], (f.angle_up - f.angle_down).to_degrees()))
}

/// Mods' screen effects and camera overlays (`block_screen_effect_draw`), drawn by the renderer
/// at [`eye_screen`]'s size after the eyes, onto both eyes this frame: `multiply` (vignette
/// blend, white is none) first, then `over` (premultiplied alpha).
pub fn overlay_eyes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    over: &wgpu::Texture,
    multiply: &wgpu::Texture,
) {
    let mut lock = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(run) = lock.as_mut() else {
        return;
    };
    if HEAD_IN_BLOCK.load(std::sync::atomic::Ordering::Relaxed) || run.eye_ready != [true, true] {
        return;
    }
    let Some(format) = run.eye_held[0].as_ref().map(wgpu::Texture::format) else {
        return;
    };
    if run.eye_overlay.as_ref().is_none_or(|m| m.0.format != format) {
        let multiply_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::Src,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        run.eye_overlay = Some((
            Mixed::new(device, format, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)),
            Mixed::new(device, format, Some(multiply_blend)),
        ));
    }
    // Mode 3 reads no depth, but the layout binds one.
    let depth = run.overlay_depth.get_or_insert_with(|| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon VR screen effects depth"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    });
    let depth = depth.create_view(&Default::default());
    let (over_mix, multiply_mix) = run.eye_overlay.as_ref().expect("made above");
    for held in run.eye_held.iter().flatten() {
        multiply_mix.over(device, encoder, multiply, &depth, held);
        over_mix.over(device, encoder, over, &depth, held);
    }
    // The first-person mirror took the left eye before these; again, with them.
    run.mirror_eye(device, queue, encoder, &depth);
}

/// This frame's face effect (`None`: none).
pub fn set_eye_tint(tint: Option<([f32; 4], f32)>) {
    *EYE_TINT.lock().unwrap_or_else(|e| e.into_inner()) = tint;
}

const TINT_WGSL: &str = r#"
struct Tint { color: vec4<f32>, vignette: vec4<f32> };
@group(0) @binding(0) var<uniform> tint: Tint;
struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = p * 2.0 - 1.0;
    return o;
}
@fragment fn fs(o: Out) -> @location(0) vec4<f32> {
    let edge = smoothstep(0.35, 1.1, length(o.uv));
    let a = tint.color.a * mix(1.0, edge, tint.vignette.x);
    return vec4<f32>(tint.color.rgb, a);
}
"#;

struct Tint {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl Tint {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon VR face effect"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon VR face effect"),
            source: wgpu::ShaderSource::Wgsl(TINT_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon VR face effect"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Horizon VR face effect"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon VR face effect"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon VR face effect"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        Self { format, pipeline, group, uniform }
    }

    fn draw(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture, tint: ([f32; 4], f32)) {
        let mut data = [0f32; 8];
        data[..4].copy_from_slice(&tint.0);
        data[4] = tint.1;
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        queue.write_buffer(&self.uniform, 0, &bytes);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon VR face effect"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

struct Mixed {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    out: Option<wgpu::Texture>,
}

const MIXED_WGSL: &str = r#"
@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var<uniform> u: vec4<f32>;
struct O {
    @builtin(position) p: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) mode: u32,
}
@vertex fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> O {
    let uv = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
    var o: O;
    o.p = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    o.uv = uv;
    o.mode = i;
    return o;
}
@fragment fn fs(o: O) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(color_tex));
    let px = vec2<i32>(clamp(o.uv, vec2<f32>(0.0), vec2<f32>(0.9999)) * dims);
    if (o.mode == 2u) {
        // The first-person mirror: the eye's middle, cropped to the window.
        let uv = vec2<f32>(0.5) + (o.uv - vec2<f32>(0.5)) * u.yz;
        let q = vec2<i32>(clamp(uv, vec2<f32>(0.0), vec2<f32>(0.9999)) * dims);
        return vec4<f32>(textureLoad(color_tex, q, 0).rgb, 1.0);
    }
    let c = textureLoad(color_tex, px, 0);
    if (o.mode == 3u) {
        // The screen effects over an eye, as drawn (premultiplied).
        return c;
    }
    if (o.mode == 0u) {
        return vec4<f32>(c.rgb, 1.0);
    }
    // Reversed depth: bigger is nearer. Nearer than the player is the foreground.
    if (textureLoad(depth_tex, px, 0) > u.x) {
        return vec4<f32>(c.rgb, 1.0);
    }
    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
}
"#;

impl Mixed {
    /// Lays `src` (premultiplied) over all of `target`.
    #[allow(clippy::too_many_arguments)]
    fn over(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Texture,
        depth: &wgpu::TextureView,
        target: &wgpu::Texture,
    ) {
        let src_view = src.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon VR screen effects"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 2, resource: self.uniform.as_entire_binding() },
            ],
        });
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon VR screen effects"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 3..4);
    }

    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon VR mixed reality"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon VR mixed reality"),
            source: wgpu::ShaderSource::Wgsl(MIXED_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon VR mixed reality"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Horizon VR mixed reality"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon VR mixed reality threshold"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { format, pipeline, layout, uniform, out: None }
    }

    /// Draws the window's picture (`size`) from the camera's colour and depth; `threshold` is
    /// the player's depth in the camera.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::Texture,
        depth: &wgpu::TextureView,
        threshold: f32,
        size: [u32; 2],
        crop: Option<[f32; 2]>,
    ) {
        if self.out.as_ref().is_none_or(|t| [t.width(), t.height()] != size) {
            self.out = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon VR mixed reality picture"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        queue.write_buffer(&self.uniform, 0, &bytemuck_f32(&[threshold, crop.map_or(1.0, |c| c[0]), crop.map_or(1.0, |c| c[1]), 0.0]));
        let color_view = color.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon VR mixed reality"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 2, resource: self.uniform.as_entire_binding() },
            ],
        });
        let out = self.out.as_ref().expect("made above");
        let view = out.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon VR mixed reality"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        if crop.is_some() {
            pass.draw(0..3, 2..3);
            return;
        }
        let half = size[0] / 2;
        pass.set_viewport(0.0, 0.0, half as f32, size[1] as f32, 0.0, 1.0);
        pass.draw(0..3, 0..1);
        pass.set_viewport(half as f32, 0.0, (size[0] - half) as f32, size[1] as f32, 0.0, 1.0);
        pass.draw(0..3, 1..2);
    }
}

fn bytemuck_f32(v: &[f32; 4]) -> [u8; 16] {
    let mut out = [0; 16];
    for (i, f) in v.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

static REVERSE_HANDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `reverseHands`: left-handed, the left controller is the main hand.
pub fn set_reverse_hands(on: bool) {
    REVERSE_HANDS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn reversed() -> bool {
    REVERSE_HANDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// `HUDLock.HEAD`: `hudDistance` ahead of the head, as wide as the menu panel.
const HUD_DISTANCE: f32 = 1.25;

static HUD_HEAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `vrHudLockMode`: HEAD (true) or WRIST.
pub fn set_hud_head(head: bool) {
    HUD_HEAD.store(head, std::sync::atomic::Ordering::Relaxed);
}

fn hud_width() -> f32 {
    if HUD_HEAD.load(std::sync::atomic::Ordering::Relaxed) { GUI_WIDTH } else { HUD_WIDTH }
}

static CAMERA_FOV_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x428C_0000); // 70.0

/// `handCameraFov`, degrees.
pub fn set_camera_fov(fov: f32) {
    CAMERA_FOV_BITS.store(fov.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Whether the game shows its HUD this frame (in a world, no menu); it then draws the HUD over
/// a cleared picture (`hud_wanted`) for the wrist panel.
pub fn set_hud(shown: bool) {
    GUI.lock().unwrap_or_else(|e| e.into_inner()).hud = shown;
}

fn hud_shown() -> bool {
    let g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    g.hud && !g.open
}

/// Whether the renderer should clear the world from the window's picture before the HUD, so the
/// HUD's picture has only the HUD (VR in 3D, in a world, no menu).
pub fn hud_wanted() -> bool {
    !screen_mode()
        && hud_shown()
        && RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|r| r.active)
}

static IN_WORLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The game is in a world (not the title screen): a menu's panel then shows the menu alone.
pub fn set_in_world(in_world: bool) {
    IN_WORLD.store(in_world, std::sync::atomic::Ordering::Relaxed);
    // A face effect belongs to a world: none on the title screen or while loading.
    if !in_world {
        set_eye_tint(None);
    }
}

/// Whether the renderer should clear the world from the window's picture before the GUI: the
/// wrist HUD, or a menu over a world (Vivecraft's GUI panel shows the menu, with the headset's
/// own world behind it, not the window's copy of the world).
pub fn panel_clear_wanted() -> bool {
    let menu = GUI.lock().unwrap_or_else(|e| e.into_inner()).open
        && IN_WORLD.load(std::sync::atomic::Ordering::Relaxed);
    !screen_mode()
        && (hud_shown() || menu)
        && RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|r| r.active)
}

/// The panel's width in the room (meters); its height follows the game window's shape.
const GUI_WIDTH: f32 = 1.3;
/// How far ahead of the head it opens.
const GUI_DISTANCE: f32 = 1.1;

/// Whether a menu is open (the game tells each frame): the panel shows while it is.
pub fn set_gui_open(open: bool) {
    let mut gui = GUI.lock().unwrap_or_else(|e| e.into_inner());
    // A menu opening is placed afresh in the gaze, not where the wrist HUD last was.
    if open && !gui.open {
        gui.panel = None;
    }
    gui.open = open;
}

pub fn gui_open() -> bool {
    GUI.lock().unwrap_or_else(|e| e.into_inner()).open
}

/// Where the laser points on the menu, 0..1 across and down.
pub fn gui_pointer() -> Option<[f32; 2]> {
    GUI.lock().unwrap_or_else(|e| e.into_inner()).pointer
}

impl Running {
    /// The game's picture for the panel (`held`), copied with this frame's commands.
    fn keep_frame(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
    ) {
        let size = frame.size();
        let reuse = self
            .held
            .as_ref()
            .is_some_and(|h| h.size() == size && h.format() == frame.format());
        if !reuse {
            self.held = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon VR menu"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: frame.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let held = self.held.as_ref().expect("made above");
        encoder.copy_texture_to_texture(frame.as_image_copy(), held.as_image_copy(), size);
    }

    fn gui_panel_ready(&self) -> bool {
        self.held.is_some()
            && GUI
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .panel
                .is_some()
    }

    fn panel_size(&self) -> [f32; 2] {
        let aspect = self
            .held
            .as_ref()
            .map_or(9.0 / 16.0, |h| h.height() as f32 / h.width().max(1) as f32);
        let width = if hud_shown() { hud_width() } else { GUI_WIDTH };
        [width, width * aspect]
    }

    /// Opens the panel ahead of the head, level, facing it; finds where the laser meets it.
    fn place_gui(&mut self) {
        let size = self.panel_size();
        let mut gui = GUI.lock().unwrap_or_else(|e| e.into_inner());
        gui.size = size;
        if !gui.open {
            gui.panel = None;
            gui.pointer = None;
            if gui.hud && HUD_HEAD.load(std::sync::atomic::Ordering::Relaxed) {
                let hmd = self.hmd;
                gui.panel = Some((hmd.translation() + hmd.direction() * HUD_DISTANCE, hmd.rotation()));
            } else if gui.hud {
                // `GuiHandler`'s WRIST lock: on the inside of the offhand's wrist, its top toward
                // the fingers, facing the body.
                let grip = POSES
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .and_then(|p| p.grip[1]);
                if let Some(grip) = grip {
                    let r = grip.rotation();
                    let turn = glam::Quat::from_mat3(&glam::Mat3::from_cols(
                        r * glam::Vec3::NEG_Y,
                        r * glam::Vec3::NEG_Z,
                        r * glam::Vec3::X,
                    ))
                    .normalize();
                    // The hotbar (the picture's bottom edge) at the wrist, the rest toward the
                    // fingers, so it sits on the hand and not down the forearm.
                    let centre = grip.translation()
                        + r * glam::Vec3::new(0.06, 0.0, 0.04)
                        + turn * glam::Vec3::Y * (size[1] / 2.0 - 0.03);
                    gui.panel = Some((centre, turn));
                }
            }
            return;
        }
        let hmd = self.hmd;
        if gui.panel.is_none() {
            // `GuiHandler.onScreenChanged`: 1 m along the head's look (up and down too, so it
            // opens in the middle of the view), facing the head (`rotationY(yaw).rotateX(pitch)`).
            let look = hmd.direction().normalize_or(glam::Vec3::NEG_Z);
            let centre = hmd.translation() + look * GUI_DISTANCE;
            let pitch = look.y.clamp(-1.0, 1.0).asin();
            let yaw = std::f32::consts::PI + look.x.atan2(look.z);
            let turn = glam::Quat::from_rotation_y(yaw) * glam::Quat::from_rotation_x(pitch);
            gui.panel = Some((centre, turn));
        }
        let (centre, turn) = gui.panel.expect("placed");
        gui.pointer = self.hands_aim(0).and_then(|aim| {
            let normal = turn * glam::Vec3::Z;
            let dir = aim.direction();
            let denom = dir.dot(normal);
            if denom.abs() < 1e-5 {
                return None;
            }
            let t = (centre - aim.translation()).dot(normal) / denom;
            if t < 0.0 {
                return None;
            }
            let hit = aim.translation() + dir * t;
            let local = turn.inverse() * (hit - centre);
            let u = local.x / size[0] + 0.5;
            let v = 0.5 - local.y / size[1];
            ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some([u, v])
        });
    }

    /// The main (0) or offhand (1) controller's aim in the room.
    fn hands_aim(&self, c: usize) -> Option<Pose> {
        POSES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .and_then(|p| p.aim[c])
    }

    /// The panel's swapchain, sized to the menu's picture.
    fn prepare_panel(&mut self, device: &wgpu::Device) -> Result<bool, String> {
        let Some(held) = self.held.as_ref() else {
            return Ok(false);
        };
        let size = [held.width(), held.height()];
        let format = held.format();
        if self
            .screen
            .as_ref()
            .is_none_or(|s| s.size != size || s.format != format)
        {
            self.screen = Some(self.make_screen(device, size, format)?);
        }
        Ok(true)
    }
}

/// Ends the frame with the menu panel over the world (`projection`, when the eyes drew).
#[allow(clippy::too_many_arguments)]
fn end_panel(
    stream: &mut xr::FrameStream<xr::Vulkan>,
    space: &xr::Space,
    screen: &mut Screen,
    held: &wgpu::Texture,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    time: xr::Time,
    before: &[&xr::CompositionLayerBase<'_, xr::Vulkan>],
) -> Result<(), String> {
    {
        let size = [held.width(), held.height()];
        let format = held.format();

        let index = screen
            .swapchain
            .acquire_image()
            .map_err(|e| e.to_string())?;
        screen
            .swapchain
            .wait_image(xr::Duration::INFINITE)
            .map_err(|e| e.to_string())?;
        let image = &screen.images[index as usize];
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(held.as_image_copy(), image.as_image_copy(), held.size());
        queue.submit([encoder.finish()]);
        // The laser's dot on the menu.
        let (pointer, panel) = {
            let gui = GUI.lock().unwrap_or_else(|e| e.into_inner());
            (gui.pointer, gui.panel)
        };
        if let Some([u, v]) = pointer {
            let dot = (size[0] / 160).max(6);
            let x = ((u * size[0] as f32) as u32)
                .saturating_sub(dot / 2)
                .min(size[0].saturating_sub(dot));
            let y = ((v * size[1] as f32) as u32)
                .saturating_sub(dot / 2)
                .min(size[1].saturating_sub(dot));
            let red: [u8; 4] = match format {
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
                    [0, 0, 255, 255]
                }
                _ => [255, 0, 0, 255],
            };
            if matches!(
                format,
                wgpu::TextureFormat::Bgra8Unorm
                    | wgpu::TextureFormat::Bgra8UnormSrgb
                    | wgpu::TextureFormat::Rgba8Unorm
                    | wgpu::TextureFormat::Rgba8UnormSrgb
            ) {
                let pixels: Vec<u8> = std::iter::repeat_n(red, (dot * dot) as usize)
                    .flatten()
                    .collect();
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: image,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(dot * 4),
                        rows_per_image: Some(dot),
                    },
                    wgpu::Extent3d {
                        width: dot,
                        height: dot,
                        depth_or_array_layers: 1,
                    },
                );
                queue.submit(std::iter::empty());
            }
        }
        screen
            .swapchain
            .release_image()
            .map_err(|e| e.to_string())?;
        let Some((centre, turn)) = panel else {
            return stream
                .end(time, xr::EnvironmentBlendMode::OPAQUE, before)
                .map_err(|e| e.to_string());
        };
        let hud = hud_shown();
        let width = if hud { hud_width() } else { GUI_WIDTH };
        let [w, h] = [width, width * size[1] as f32 / size[0].max(1) as f32];
        let quad = xr::CompositionLayerQuad::new()
            .space(space)
            // See-through where the picture was cleared (`panel_clear_wanted`): the HUD, a menu in a world.
            .layer_flags(if hud || IN_WORLD.load(std::sync::atomic::Ordering::Relaxed) {
                xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA
                    | xr::CompositionLayerFlags::UNPREMULTIPLIED_ALPHA
            } else {
                xr::CompositionLayerFlags::EMPTY
            })
            .eye_visibility(xr::EyeVisibility::BOTH)
            .sub_image(
                xr::SwapchainSubImage::new()
                    .swapchain(&screen.swapchain)
                    .image_rect(xr::Rect2Di {
                        offset: xr::Offset2Di { x: 0, y: 0 },
                        extent: xr::Extent2Di {
                            width: size[0] as i32,
                            height: size[1] as i32,
                        },
                    }),
            )
            .pose(xr::Posef {
                orientation: xr::Quaternionf {
                    x: turn.x,
                    y: turn.y,
                    z: turn.z,
                    w: turn.w,
                },
                position: xr::Vector3f {
                    x: centre.x,
                    y: centre.y,
                    z: centre.z,
                },
            })
            .size(xr::Extent2Df {
                width: w,
                height: h,
            });
        let mut layers = before.to_vec();
        layers.push(&quad);
        stream
            .end(time, xr::EnvironmentBlendMode::OPAQUE, &layers)
            .map_err(|e| e.to_string())
    }
}
