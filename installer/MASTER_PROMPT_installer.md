```markdown
You are working in the Horizon VR repo on Windows. You may edit files, build with Cargo, and run the PowerShell tests.

Read these files completely before changing anything:

- `installer/app/src/main.rs`
- `installer/app/src/gui.rs`
- `installer/app/Cargo.toml`
- `installer/build-release.ps1`
- `installer/test-installer.ps1`
- `installer/HorizonVR-Installer.ps1`
- `installer/Play Horizon VR.bat`

Context:

- The Rust installer exe is currently about 2.3 MB.
- The player download zip is about 57 MB because `files\` contains `horizon-client.exe` (~156 MB unpacked), `horizon-prepare.exe`, `openxr_loader.dll`, and exporter assets.
- The installer copies `files\` to the selected install folder, prepares game files from the player’s own Minecraft 26.3, creates shortcuts, and imports the game into `horizon-launcher.exe` through its CLI.
- Releases are GitHub releases for `v1nx0/horizon-vivecraft-vr`, discovered via the latest-release API.
- Existing/old installers must remain able to update: they specifically download `HorizonVR-Installer.zip`, expect `HorizonVR-Installer.ps1`, and expect `files\horizon-client.exe` inside it.

Your goal has two parts: thoroughly harden the installer, and turn the normal player download into a <=5 MB web-installer exe.

Work rules:

- Make the smallest complete change that works. Do not redesign unrelated installer behavior.
- Reproduce each bug before fixing it. For every real bug you fix, add a focused failing automated test first, then implement the fix.
- Do not weaken integrity checks, error handling, or backwards compatibility merely to simplify implementation.
- Never touch the user’s real install (`%LOCALAPPDATA%\HorizonVR`) or their real Horizon launcher/data during tests.
- Tests must use isolated temporary fixtures and set `HORIZONVR_LAUNCHER`, `HORIZONVR_LAUNCHER_DATA`, and `HORIZONVR_RELEASES`. If shortcut locations need isolation, add a clearly scoped test override rather than writing to the real Desktop or Start Menu.
- Do not depend on the user’s real Minecraft installation, real server jar, or real launcher for test success. Build minimal fake launcher/Minecraft/prepare fixtures as needed.
- Do not push. Commit locally only after the implementation and tests are green.
- Preserve user data: worlds, settings, unrelated mods/files, and launcher state must never be deleted or overwritten by installer failure paths.
- At the end, report the concrete bugs found, tests added, implementation choices, release assets produced, commands run, test results, output sizes, and the local commit hash.

## Part 1: Installer bug hunt and hardening

Audit all installer paths, including GUI and silent modes, install, reinstall, repair, update, check-on-launch, uninstall, launcher integration, legacy shim execution, build/release packaging, and test isolation.

Do not limit yourself to this list, but explicitly investigate these concrete risk areas in the existing code:

- Path correctness in `full_path`, `here`, `default_install_dir`, `find_minecraft`, `find_server_jar`, `copy_all`, shortcut creation, launcher invocation, and PowerShell/batch argument passing:
  - spaces;
  - Unicode/non-ASCII names;
  - OneDrive-redirected Desktop;
  - trailing backslashes and quotes;
  - drive roots and UNC paths;
  - 8.3 short paths versus long paths;
  - paths containing `&`, `%`, quotes, or other command-shell-sensitive characters.
- Running the installed installer from the destination folder while it replaces or removes itself:
  - `install`, `update`, and `uninstall`;
  - rename collisions in `%TEMP%`;
  - antivirus/indexer/file-lock races;
  - self-replacement failure or a partially updated install.
- Download/update robustness in `agent`, `get_latest`, `download`, and `get_release`:
  - offline/no network;
  - DNS/TLS/timeout failures;
  - GitHub API rate-limit responses and malformed/non-JSON responses;
  - missing release assets or absent/malformed asset digest;
  - redirects;
  - incomplete/truncated downloads;
  - corrupt zips;
  - unsafe archive paths / zip-slip;
  - checksum failures;
  - cleanup and retry behavior.
- Atomicity and recovery:
  - a failure while copying/removing game files;
  - preserving a runnable prior installation where practical;
  - never reporting “Done” after a failed copy, preparation, shortcut, or launcher action;
  - stale temp data and concurrent installer processes.
- `run` and launcher interactions:
  - stdout/stderr draining, invalid UTF-8, long/no-newline output, thread failure/deadlock;
  - executable missing, old launcher CLI, nonzero launcher exit, invalid/changed `--json` responses;
  - selected-version restoration and imported-version cleanup;
  - launcher absent should remain nonfatal where intended, but clearly reported;
  - never edit real launcher settings/data in tests.
- `assert_closed`, recursive deletion, `INSTALLED`, `remove`, and uninstall:
  - missing files and junction/symlink/reparse-point safety;
  - path traversal or deleting outside the install folder;
  - locks and actionable error text;
  - user saves/options and unrelated files surviving.
- `gui.rs`:
  - worker-thread/message ownership and `PostMessageW` failure;
  - messages posted after the window is destroyed;
  - leaked boxed payloads;
  - stale status refresh overwriting newer UI state;
  - enabling/disabling buttons correctly;
  - panic/unsafe assumptions around global `static mut UI`;
  - clear GUI-visible errors and log output;
  - browsing Unicode paths.
- CLI parsing, return codes, silent-mode diagnostics, `attach_console`, `--check`, and errors that are currently swallowed or too vague.
- Security and correctness of unsafe Win32 calls, pointer ownership, handle/null/error checking, and shell/launcher command construction.
- Build script/test script flaws, especially the current test script’s dependence on real user folders and launcher/Minecraft state.

For each confirmed issue:

1. Add a test that demonstrates the failure before the fix.
2. Implement the smallest root-cause fix.
3. Make errors actionable, naming the failed stage and useful path/status without exposing irrelevant internals.
4. Keep behavior compatible with existing installs unless an explicit migration is necessary.

Use unit tests for deterministic logic and hermetic integration tests for release/package/install/update behavior. Avoid fragile timing-only tests. If a case cannot be reliably automated, explain why and add the closest deterministic coverage plus a concise manual test procedure.

## Part 2: <=5 MB web-installer design and implementation

Change the player-facing release flow so the normal download is a small web-installer exe of 5 MB or less. It must download the large payload only during installation/update.

Implement this practical release layout:

1. `HorizonVR-Installer.exe`
   - The player-facing web-installer asset.
   - Must be <=5 MB.
   - It contains installer logic/UI only, not `horizon-client.exe`, exporters, or the large game payload.

2. `HorizonVR-Payload.zip`
   - The large, versioned install payload.
   - Contains the release’s `files\` tree: client, prepare executable, OpenXR loader, exporters, lock/version/base files, launcher batch, and the installer copy needed in the installed folder.
   - The installed copy must be capable of later update/repair/install-from-installed-folder behavior.

3. A payload SHA-256 source that the installer verifies before extraction.
   - Prefer the GitHub release asset digest when present, but provide a reliable explicit sidecar/checksum asset fallback if GitHub’s API response lacks `digest`.
   - Treat a missing or malformed required hash as a hard failure. Do not install unverified payloads.

4. `HorizonVR-Installer.zip`
   - Keep producing this large legacy compatibility asset.
   - It must retain the exact old-compatible layout expected by released old installers:
     - `HorizonVR-Installer.ps1` at the expected location;
     - `files\horizon-client.exe`;
     - all other required legacy payload content.
   - Do not claim this asset is small. It exists so old installers’ Update continues to work.
   - The normal release instructions/player-facing download must point to the small `HorizonVR-Installer.exe`, not this legacy zip.

The current installer’s own update path may migrate to the payload asset. Old released installers must continue using the legacy zip unchanged. Be explicit in code and tests about the two paths.

### Download requirements

The web installer must:

- Fetch release metadata with a stable User-Agent and useful handling for offline, rate-limited, malformed, and missing-asset responses.
- Download `HorizonVR-Payload.zip` with visible GUI progress and useful silent-mode progress text.
- Resume an interrupted download using a `.part` file and HTTP Range requests when the server supports them.
- Correctly handle servers that ignore Range and return a full `200`; restart safely rather than appending corrupt content.
- Validate response status/content-range before appending.
- Verify the completed SHA-256 before extraction.
- Keep an interrupted `.part` file for retry when safe; remove corrupt completed downloads/extractions.
- Extract only after verification, reject archive traversal, and avoid leaving a partially installed game due to a failed extraction/copy.
- Give concise, actionable errors such as “No internet connection,” “GitHub rate limit reached,” “download interrupted; retry will resume,” “payload checksum did not match,” or “another program is locking …”.
- Keep the UI responsive. Integrate progress through the existing worker/message model or the smallest safe equivalent.

Use a deterministic local HTTP test server for integration tests. It must cover at least:

- correct payload install;
- checksum mismatch;
- interrupted download followed by resume;
- Range ignored/full-response fallback;
- missing hash asset/digest;
- offline/latest API failure;
- payload missing expected files;
- legacy zip remains valid for old-installer update expectations;
- installed web installer can update using the new payload design.

### Size work

Keep the release binary <=5 MB and document its measured size in the final report.

Review `installer/app/Cargo.toml` and use only justified size reductions. Consider:

- `opt-level = "z"`;
- `panic = "abort"`;
- LTO/strip/codegen-unit settings appropriate for a small Windows release binary;
- removing heavy dependencies such as `rfd` or `ureq`/Rustls if small native Win32/standard-library alternatives can cover the exact needed behavior;
- feature trimming.

Do not replace reliable behavior with a size trick. Measure the actual built `HorizonVR-Installer.exe`; the 5 MB requirement is for that exe, not a compressed zip estimate.

### Build and test script changes

Update `installer/build-release.ps1` to:

- build the small web installer;
- assemble the payload asset;
- generate and package the payload SHA-256 sidecar/metadata;
- assemble the separate legacy `HorizonVR-Installer.zip` with the historical structure;
- validate required files before packaging;
- print each produced asset and its size/hash;
- preserve the existing release-version guard behavior, while handling GitHub unavailability sensibly.

Update `installer/test-installer.ps1` so it is hermetic and verifies both the small-web-installer and legacy-package flows. It must not invoke or mutate the real launcher, launcher data, `%LOCALAPPDATA%\HorizonVR`, Desktop, Start Menu, Minecraft directory, or Downloads folder.

Run all of the following before committing:

```powershell
cargo test --release --manifest-path installer/app/Cargo.toml
powershell -NoProfile -ExecutionPolicy Bypass -File installer/build-release.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File installer/test-installer.ps1
```

If a command cannot run because of a genuine environment prerequisite, do not fake success: state exactly what blocked it, preserve the test evidence, and run every remaining applicable check.

Finally, make one local commit only. Do not push.
```