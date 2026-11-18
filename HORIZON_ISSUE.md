# Mod API: what a VR native mod (Vivecraft port) needs

## Summary

I ported [VivecraftMod](https://github.com/Vivecraft/VivecraftMod) (LGPL-3.0) to Horizon: OpenXR
headset and controllers, eye rendering, menus on panels in 3D, roomscale movement, and Vivecraft's
server protocol. It works today as a patch on Horizon `c07bfa7`:
https://github.com/v1nx0/horizon-vivecraft-vr

I'd like to ship it as a native `.hmod` instead of a patched build. Most of it already fits:
the `horizon-client` changes can become `[[native.mixin]]` and `[[native.module]]` rows, and
the OpenXR code can move into the mod's own crate (`openxr` from crates.io). Three things are out
of reach for a mod now, per the native route docs. This issue lists them with the exact targets.

## 1. Renderer functions a mixin can't reach

The port changes these functions in `horizon-client-render`. A mixin can only replace functions
below the game crate when they carry `#[horizon_mod_api_macros::replaceable]`
(`E_MIXIN_NOT_REPLACEABLE`), and these don't.

| File | Function | What VR needs there |
|---|---|---|
| `src/gfx/surface_setup.rs` | `open_adapter` | Pick the GPU the OpenXR runtime asks for (Vulkan), before the device is made |
| `src/gfx.rs` | `open_backend` | Same: the adapter OpenXR drives the headset with |
| `src/gfx.rs` | `Renderer::draw_frame` | Draw the world once per eye; clear the window to see-through for the HUD panel; mirror an eye to the window |
| `src/gfx.rs` | `Renderer::draw_overlay` | Draw screen effects (tints, vignette, mod overlays) into each eye instead of the window |
| `src/gfx.rs` | `Renderer::camera_frustum` | Use each eye's own projection and pose |
| `src/gfx/view_sections.rs` | `Renderer::ask_world_views`, `Renderer::walk_world_views` | Section culling for both eyes' frusta |
| `src/gfx/world_views.rs` | `Renderer::draw_world_views`, `Renderer::draw_world_view` | Render a world view into an OpenXR swapchain image |
| `src/gfx/panorama.rs` | `projection`, `Panorama::cube` | The title panorama all around the headset |
| `src/ui.rs` | `menu_background` | Menu background on the 3D panel |

**Ask:** mark these `replaceable`. Better and smaller: one published point that lets a mod render
the world into extra views. For example, a hook after the frame's world pass, with the device,
queue, encoder and the per-view camera (pose, projection, target texture). Then a VR mod needs no
renderer replacements at all.

## 2. Fields a mod can't add

`[[native.state]]` is refused, and an attachment only covers entities and blocks. The port adds:

| Type | Field | Use |
|---|---|---|
| `horizon_client_render::gfx::Renderer` | `vr_screen: Option<[wgpu::Texture; 2]>` | Per-eye texture for screen effects |
| `gfx::world_views::Lists` | eye draw lists | One draw list per eye |
| `horizon_client::mobs::Extra` | `vr_leg_angles` | Other VR players' legs and body from their trackers |
| `horizon_client::motion` (player motion state) | `vr_crawl` | Roomscale crawling |

**Ask:** a way to keep per-renderer state in a mod. The renderer fields could live in the mod's own
statics, if the render hook above passes the device and queue. Player model state could use an
attachment on remote players, if attachments are available client-side for model posing.

## 3. A native library beside the game

OpenXR loads through `openxr_loader.dll`, which the port ships next to `horizon-client.exe`.

**Ask:** say whether a native mod may ship a DLL the game loads (and where it goes). If not, I'll
build the loader into the mod statically with the `openxr` crate's `static` feature, if the
launcher's pinned toolchain allows a C/C++ build step (it needs CMake and a C++ compiler).

## Already fine, for reference

These can already be done in a native mod, so they need nothing:

- Input, camera and movement in `horizon-client` (`play_keys`, `pointer_capture`, `motion`, `pose`, ...): mixins.
- Item use and hands (`items/hand.rs`, `using_item.rs`, `clip_ray.rs`): mixins.
- Vivecraft's `vivecraft:data` channel and arm poses (`network/remote`, `interface/perspective.rs`): mixins.
- All VR game modules: the mod's crate and `[[native.module]]` rows.

## Why it matters

Vivecraft is a popular Minecraft mod. With these three things Horizon could have VR as
an ordinary mod from the mod browser, with no patched build. I'm happy to test any API change
against the port and to make the PR for the `replaceable` markers if that's the preferred route.
