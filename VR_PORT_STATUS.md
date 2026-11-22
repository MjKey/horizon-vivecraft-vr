# Horizon VR: Vivecraft port status

Vivecraft (LGPL-3.0, github.com/Vivecraft/VivecraftMod) ported into Horizon (LGPL-3.0). Every ported file
names the Vivecraft file it comes from.

## How to test
1. Start SteamVR and connect the Quest (Steam Link). Turn **HDR off** in Horizon's video settings.
2. Double-click **Play Horizon VR.bat** (or run it with `--world "New World"` to go straight into a world).
3. Without VR (`HORIZON_VR` unset) the game runs exactly as before.

### Testing without the headset: `HORIZON_VR=sim`
Run `Test Horizon VR (no headset).bat`. No SteamVR needed: a simulated standing player whose head
and both hands follow the mouse look. Everything in the game's Vivecraft port runs (arms and items at
the controllers, controller aiming, roomscale room, climbing, trackers), drawn on the normal window.
Gestures (swinging, bow drawing, eating) need real controllers. For scripted checks,
`sim_test.sh` starts it under `horizon-ctl` (then `horizon-ctl ... state | screenshot | hold w 1`).

Log lines start with `Horizon VR:`. The game's log is in the console; SteamVR's in `Steam\logs\vrserver.txt`.

## Ported (built, NOT yet tested in the headset)
| Vivecraft | What it does | Where |
|---|---|---|
| VRData / VRPlayer | Room in the world (origin, rotation, scale); roomscale walking with step-up, room follows the player, falling | `game/vr/vr_player.rs` |
| Snap turn | Right stick turns 45Â° around your head | `vr_player.rs` |
| Controller aim | Breaking/placing/using aims from the right controller's real ray | `game.rs` `aim()` |
| Look override | Player looks where the right hand aims (head when swimming/gliding) | `vr_player.rs` |
| Free move | Left stick walks toward where the **left** controller points | `vr_player.rs` |
| SwingTracker | Swing a hand/tool to mine (faster = more damage), hit mobs, punch doors open | `vr_trackers.rs` |
| JumpTracker | Jump in real life to jump | `vr_trackers.rs` |
| SneakTracker | Crouch in real life to sneak | `vr_trackers.rs` |
| ClimbTracker | Grab ladders/vines automatically, pull yourself up, climb onto the top | `vr_trackers.rs` |
| BowTracker + BowModule | Bow in left hand: nock with right hand at the string, hold trigger, pull back, let go | `vr_trackers.rs` |
| EatingTracker | Hold food/drink at your mouth to eat/drink (tip drinks) | `vr_trackers.rs` |
| InteractTracker | Hand in a lever/button/door/chest/etc. + trigger = use it; hand in a mob + trigger = interact | `vr_trackers.rs` |
| BackpackTracker | Right hand over shoulder = quick swap to slot 1 and back; left = swap offhand | `vr_trackers.rs` |
| SwimTracker | Breaststroke to swim | `vr_trackers.rs` |
| Haptics | Controller buzz on hits, grabs, bow draw, eating | `horizon-vr` `haptic()` |
| Arms + held items | Your skin's arms and the held item at each controller, Vivecraft's per-item poses | `game/vr/vr_render.rs`, `frame_geometry.rs` |
| GuiHandler HUD (WRIST lock) | Hotbar, health, hunger on the inside of your left wrist while playing (the desktop window shows only the HUD then) | `horizon-vr` `set_hud`, `gfx.rs` |
| InteractiveHotbarModule | Touch a wrist hotbar slot with the main hand (buzz), trigger selects it; the slot left of the bar swaps the offhand | `vr_trackers.rs`, `horizon-vr` `wrist_hud` |
| TeleportTracker | Push the right stick forward: a falling arc from the left controller (blue = can land, red = can't); let go to teleport. Survival limits 1 up / 4 down / 16 across | `game/vr/vr_teleport.rs` |
| RowTracker + AbstractBoatMixin | In a boat: swing both arms to row (one arm turns), stick turns by how far it's pushed, forward drives toward the left controller (tank controls) | `game/vr/vr_vehicle.rs`, `boats/mod.rs` |
| VehicleTracker | Riding a boat/horse/minecart: the room turns with the vehicle | `vr_player.rs` |
| TelescopeTracker | Hold a spyglass's end up to an eye: a 7Â° zoomed view shows where it points (round, as Vivecraft) | `horizon-vr` `lens_view`, `view_sections.rs` |
| RadialHandler (hold mode) | Hold B: a ring of 9 labels in front of the right controller; point at one, let go. Drop, chat, rotate left/right, pick block, handheld camera, player list (stays shown until toggled), quick torch, Mods (Extras page: fullbright, zoom, FPS, coords). Let go at once = repeat the last | `game/vr/vr_radial.rs` |
| CameraTracker | Radial "Toggle Handheld Camera": a camera in front of your right hand, staying in the world, its live picture on its back. Right grip at it = grab and move; right trigger at it = photo into `screenshots/vr_camera_*.png` (HDR off) | `game/vr/vr_camera.rs`, `horizon-vr` |
| VRSettings | **VR Settings** button in the pause menu (VR only): free move type (controller / HMD / run in place), rotation increment (smooth, 10â€“180Â°), world scale, teleport on/off and survival limits, roomscale jumping/sneaking/climbing/swimming/rowing/crawling, HUD on wrist or head, radial hold/press and repeat, walk up blocks, simulate falling, camera FOV. Saved in `horizon-vr.txt` | `game/vr/vr_settings.rs` |
| Seated / reverse hands / mirror | VR Settings: **Seated Mode** (no roomscale trackers or radial, room stays on you, walk where you look, HUD on head), **Reverse Hands** (left-handed: every control and the aim move to the other hand), **Desktop Mirror**: Third Person (Camera) shows the handheld camera on the desktop; **Mixed Reality** splits it: the camera view left, and right only what is nearer the camera than you on green, for keying in OBS | `vr_settings.rs`, `horizon-vr` |
| Menu world | On the title screen the headset shows the title panorama all around you (Horizon's stand-in for Vivecraft's menu world), the menu panel in front | `world_views.rs` `draw_title_views`, `panorama.rs` |
| RunTracker | Free move type "Run In Place": swing both arms to run where they point | `game/vr/vr_run.rs` |
| CrawlTracker | Lie down to crawl (single player / hosting a LAN world; Vivecraft needs its server, a remote server can't) | `game/vr/vr_crawl.rs`, `pose.rs` |
| Climb Claws / Jump Boots | Unbreakable shears named "Climb Claws": trigger grabs any solid block. Unbreakable leather boots named "Jump Boots": hold a grip, pull down, let go to leap (single player / LAN host) | `game/vr/vr_climb_claws.rs` |
| Eye views | Particles, sun, moon, stars, clouds, rain and snow in both eyes; with packs: Horizon's native shader packs' own sky, sun/stars hiding, weather and clouds, OptiFine custom sky layers, the End sky, the dark disc | `gfx/world_views.rs` |
| GuiHandler | Menus (inventory, chests, pause, title) on a floating panel; right laser points (red dot), trigger = left click, grip = right click | `horizon-vr`, `pad_menu.rs` |

| Menu panel (GuiHandler.onScreenChanged) | Opens 1 m along your gaze (up/down too), facing you; shows only the menu (no dark backdrop, no HUD under it) | `horizon-vr` `place_gui`, `panel_clear_wanted` |
| VREffectsHelper face effects | Pumpkin, nether portal, freezing, on fire, low health (pulsing red) tint your eyes instead of drawing on the wrist HUD | `horizon-vr` `Tint`, `render_tick.rs` |
| In-world crosshair | Minecraft's `hud/crosshair` sprite lying flat on the block face the laser hits, sized with distance | `game/vr/vr_crosshair.rs` |
| Mod screen effects in VR | Mods' block screen effects (`block_screen_effect_draw`) and mod camera overlays are laid out for the eye's size and field of view, drawn offscreen and laid over both eyes in the same frame; vignette-blended ones darken the eyes as on screen (vanilla ones stay the eye tint) | `render_tick.rs`, `game.rs`, `gfx.rs` `vr_screen`, `horizon-vr` `eye_screen` / `overlay_eyes` |
| ClientNetworking | On a server with Vivecraft's plugin (`vivecraft:data`): announces the channel and the version, sends head/hand poses every tick (new and legacy packets), world scale, height, crawl, climbing, climbey jumps, direct teleports (any distance), bow draw and arrow hand; reads what the server allows (crawl, climbey + its block list, teleport on/off) and other VR players' poses. Crawl, claws, jump boots and teleport now work on such servers | `game/vr/vr_net.rs`, `remote/mod.rs` |
| ClientVRPlayers / VRPlayerModel (arms) | Other VR players' arms point at their controllers | `vr_net.rs` `vr_net_arm_angles`, `perspective.rs` |
| OSCTrackerReceiver (full body) | VR Settings â†’ **OSC Trackers**: listens on UDP 9000 (SlimeVR / VRChat OSC). Waist and feet go to the server as `ARMS_LEGS`. Roles are guessed by height (no T-pose calibration). **Untested: needs someone with trackers** | `game/vr/vr_osc.rs` |
| Debug device axes | VR Settings â†’ **Local Device Axes** / **Remote Device Axes**: red/green/blue axes on your headset and controllers, and on other VR players' | `game/vr/vr_debug.rs` |
| autoSprint | Walk stick pushed fully forward sprints | `pad/mod.rs` |
| Crawl / Climb Claws in single player | The built-in server now knows you crawl; claws and boots work in single player | `vr_crawl.rs`, `guest_body.rs`, `vr_climb_claws.rs` |

## Controls
Left stick walk Â· right stick left/right snap turn, forward teleport Â· A jump Â· B radial menu (pad B on menus) Â· X swap offhand Â· Y inventory Â· right trigger attack/use
(or click on menus) Â· left trigger use Â· grips hotbar (right grip = right click on menus) Â· left menu button pause.

## Not ported (on purpose or impossible here)
- HorseTracker: hard-coded off in Vivecraft (`isActive` returns false) and its rein positions are never filled, so a port does nothing.
- Other VR players' headset: a plain dark box (visor and strap) at their headset pose, `vr_debug.rs`; not Vivecraft's textured model. Legs reach for their foot trackers when they send full-body tracking (`vr_net_leg_angles`), and the body turns, leans and tilts with their waist tracker and the hips follow it (knees and elbows not bent); otherwise walk normally.
- Menu worlds (`MenuWorldRenderer`/`Exporter`, ~3600 lines of their own world format and renderer): the title panorama stands in.
- Vivecraft's 3D keyboard: Horizon's controller keyboard on the menu panel does the job.
- Other VR players: no headset model on the head (`HMDLayer`) and no legs from full-body trackers; arms only.
- Inside the eyes, an Iris-style `shaders/` pack's sky and a resource pack's core/sky shader: they run in the window's
  own pack passes, so the eyes show the native sky under them (the eyes draw the world without the pack's shading
  anyway).
- Crawl, climb claws, jump boots on a remote server **without** Vivecraft's plugin.
- Swords' "correct tool" check is approximate (they break only instant blocks).

## Expect on first test
Directions of turning/arm rotations may be mirrored or offset; the log and a description of what looks wrong are
enough to fix them.





- 2026-11-19: **Mods** button (9th) on the radial ring opens Horizon's Extras page: fullbright, zoom, FPS counter and the PvP HUD coordinates, each a toggle. Fullbright is no longer held off on multiplayer servers (`fullbright_held` in `interface/options.rs`, in `horizon-hooks.patch`). Built; not headset-tested.

- 2026-11-19: new world no longer black: the room is placed again on each join (`VrPlayer.joined`), it stayed where the last world left it. Pre-launch `-Check` copies Horizon's zoom.hnmod into the VR instance. Sitting players: use Seated Mode (roomscale sneak/crawl read a low head as crouching).

- 2026-11-19: the trigger on any text field (world name, mod search, inventory search) opens Horizon's on-screen keyboard (`vr_gui_frame` in `pad_menu.rs`, in the patch).

- 2026-11-19: on-screen keyboard in VR: the trigger types the key the laser points at (`Keyboard::key_at` in `osk.rs`); pointing off it closes it.

- 2026-11-23: VR Settings: **Weapon Collision** (swing mining/hitting on/off) and **Crosshair** (in-world crosshair on/off). Launcher base moved to nightly-2026-11-20-0a875da (the 6d765e0 release is gone from GitHub).
