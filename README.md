<p align="center"><img src="logo-installer.png" width="220" alt="Horizon VR logo"></p>

# Horizon Vivecraft VR

[![Download](https://img.shields.io/github/v/release/MjKey/horizon-vivecraft-vr?label=Download&color=2ea44f)](https://github.com/MjKey/horizon-vivecraft-vr/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/MjKey/horizon-vivecraft-vr/total?color=blue)](https://github.com/MjKey/horizon-vivecraft-vr/releases)

Play Minecraft 26.3 in VR on Horizon: a port of
[VivecraftMod](https://github.com/Vivecraft/VivecraftMod) and
[VR-Throwing-Extensions](https://github.com/demistorm/VR-Throwing-Extensions) using OpenXR
(SteamVR, Oculus/Meta Touch, Valve Index, HTC Vive, Windows Mixed Reality), forked and extended
from [v1nx0/horizon-vivecraft-vr](https://github.com/v1nx0/horizon-vivecraft-vr).

## Requirements

- Windows 10 or 11 and a VR headset compatible with SteamVR / OpenXR.
- Minecraft Java Edition 26.3, played once (in the Horizon launcher or the official Minecraft Launcher).
- The Minecraft 26.3 server jar (found automatically if the Horizon launcher already downloaded it, or from minecraft.net).

## Install

1. Download `HorizonVR-Installer.exe` from [Releases](https://github.com/MjKey/horizon-vivecraft-vr/releases/latest) and run it.
2. Check the install folder, Minecraft folder, and server jar path, then click **Install** (downloads `HorizonVR-Payload.zip` and prepares game assets).
3. Start SteamVR, then launch **Horizon VR** from your desktop or the Horizon launcher.

Pause menu: hold **Y** on the left controller. A short tap of **Y** opens the inventory.

## Features

- **OpenXR & Multi-Controller Support**: Auto-discovers SteamVR `openxr_loader.dll` via Windows registry + `libraryfolders.vdf`; binds Meta/Oculus Touch, Valve Index, HTC Vive, WMR, and Khronos Simple Controller profiles.
- **Asymmetric Stereo Rendering**: Uses OpenXR per-eye asymmetric FOV tangents, configurable `renderScaleFactor`, and early-Z corner masking (`stencilOn`).
- **Roomscale & Seated Comfort**: Standing height calibration (`Calibrate Height`), seated eye calibration, per-frame smooth turning with adjustable speed, walking speed multiplier, and configurable auto-sprint threshold.
- **VR Combat, AimFix & Multiplayer Protocol**: Shots from bows, crossbows, tridents, potions, pearls, and snowballs originate from hand/bow aim; VR bow headshots (3x roomscale / 2x seated); full Vivecraft C2S/S2C payloads (`HAPTIC`, `SETTING_OVERRIDE`, `SERVER_VR_CHANGES`, `DAMAGE_DIRECTION`, `DUAL_WIELDING`, `ATTACK_WHILE_BLOCKING`, `VR_SWITCHING`, `AIM_*_OVERRIDE`, `ROOMSCALE_ATTACK`, `ACTIVEHAND`).
- **3D HUD & Face Effects**: Configurable HUD scale, distance, opacity, HUD hiding, optional physical GUI poke-touch, per-overlay toggles (pumpkin, portal, fire, water, nausea, low health), inside-block blackout with wall-grazing margin, plus the 2D on-screen keyboard.
- **VR Throwing Extensions**: Grip-hold + swing-release throwing (or boosted Drop key), ballistic aim assist, thrown weapon impact damage, sharp weapon block embedding, boomerang return, and physical mid-air item catching.
- **Full-Body Trackers & Radial Menu**: OSC full-body tracking (port 9000), climbing/claws, crawling, swimming, rowing, handheld camera, and mods ring menu.

Please report bugs in [Issues](https://github.com/MjKey/horizon-vivecraft-vr/issues).

## Build from source (or via GitHub Actions)

Cloud builds & releases run automatically on GitHub Actions (`.github/workflows/release.yml`) on every `v*` tag push or manual `workflow_dispatch`.

To build locally on Windows (needs Git, Rust, and JDK 25), run `Install.bat` / `install.ps1`, which fetches the base Horizon snapshot (`horizon-source.tar.gz`), adds `crates/horizon-vr/` and `horizon-client-vr/vr/`, applies `horizon-hooks.patch`, builds with `cargo build --profile fast -p horizon-game -p horizon-prepare`, and prepares game files.

| Path | What it holds |
|---|---|
| `crates/horizon-vr/` | OpenXR runtime, controller profiles, stereo eye rendering, HUD/GUI panels |
| `horizon-client-vr/vr/` | Game VR modules (`vr_player`, `vr_trackers`, `vr_throw`, `vr_net`, `vr_settings`, etc.) |
| `horizon-hooks.patch` | Patches to Horizon's client & renderer crates |
| `installer/` | GUI installer (`app/`), `build-release.ps1`, `test-installer.ps1` |
| `.github/workflows/release.yml` | GitHub Actions workflow to test, build, and publish release assets |

## License

LGPL-3.0, like Vivecraft. See [`LICENSE.md`](LICENSE.md).
