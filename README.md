# Horizon Vivecraft VR

[![Bitcoin](https://img.shields.io/badge/Bitcoin-donate-F7931A?logo=bitcoin&logoColor=white)](#donate)
[![Solana](https://img.shields.io/badge/Solana-donate-9945FF?logo=solana&logoColor=white)](#donate)

A port of [VivecraftMod](https://github.com/Vivecraft/VivecraftMod) (LGPL-3.0) to the Horizon
Minecraft client, using OpenXR. Only the VR code is here, not Horizon itself.

## What's here

- `crates/horizon-vr/`: the OpenXR runtime crate (headset, controllers, eye rendering).
- `horizon-client-vr/vr/`: the game VR modules. They go in `crates/horizon-client/src/game/vr/`.
- `horizon-hooks.patch`: the changes to Horizon's own files that hook the VR code in.
- `VR_PORT_STATUS.md`: what is ported, how, and what is not.

## Install

Easiest way, like Vivecraft's installer: download `HorizonVR-Installer.zip` from Releases, unzip it,
and double-click `Install Horizon VR.bat`. In the window, check the install folder. It finds your Minecraft
folder by itself (the Horizon launcher's or `.minecraft`), and the 26.3 server jar when the
Horizon launcher downloaded it or it is in Downloads; otherwise download the 26.3 server jar from
minecraft.net and pick it. Click **Install**. It needs Minecraft 26.3 played
once in the Minecraft Launcher. It adds a "Horizon VR" shortcut to your desktop. No Git or Rust needed.

The zip is made with `installer/build-release.ps1` from a built Horizon checkout with this port.

From source: install [Git](https://git-scm.com/download/win), [Rust](https://rustup.rs) and
[JDK 25](https://adoptium.net), download this repo, and double-click `Install.bat`. It downloads
Horizon, adds the VR code, builds it and prepares the game files from your own Minecraft 26.3
(it asks for the 26.3 server jar). Then start SteamVR and run `Play Horizon VR.bat`.

By hand:

1. Clone Horizon.
2. Copy `crates/horizon-vr` into Horizon's `crates/`.
3. Copy `horizon-client-vr/vr` to `crates/horizon-client/src/game/vr`.
4. In the Horizon folder, run `git apply path/to/horizon-hooks.patch`.
5. Build with `cargo build --profile fast -p horizon-game`.

The patch was made against Horizon commit `c07bfa7`. A newer Horizon may need it fixed by hand.

## Status

Tested only partly in a headset. The Vivecraft server networking, other players' VR bodies and
full-body (OSC) trackers have not been tested at all.

## License

LGPL-3.0, as Vivecraft. See `LICENSE.md`. The ported code credits the Vivecraft source files it
came from.

## Donate

Bitcoin:

```
bc1q0csyzscm725lu4vwf5tsxwg5vr32pgjujtpnh8
```

Solana:

```
HMmUu1qFiWqfHiW5DMvUix5Ai9THvnC6VRwYLRM8zbbs
```
