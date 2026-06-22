# Fresh Driver

A Windows desktop app that scans your installed device drivers, finds available
updates (via **Windows Update** plus select **vendor** checks), and installs them
safely — with a System Restore point created first and per-operation UAC
elevation (the app itself never runs as admin).

Built with **Tauri v2** (Rust backend + web frontend).

---

## What it does

| Stage | How |
|-------|-----|
| **Scan** all installed drivers | WMI `Win32_PnPSignedDriver` (device, version, date, class, vendor) |
| **Find updates online** ⭐ | **Microsoft Update Catalog** — queries each device's `VEN&DEV`/`VID&PID` concurrently and reports anything newer than installed. This is the comprehensive source (catches what Windows Update won't offer) |
| **Find updates** (Windows Update) | Windows Update Agent COM API — search `IsInstalled=0 and Type='Driver'`. Conservative; only what Microsoft promotes for your machine |
| **Vendor check** (GPU) | Detects NVIDIA GPU + current driver version; links to the official download |
| **Install** | Downloads & installs the chosen Windows Update driver — elevated, one UAC prompt |
| **Safety** | Creates a `DEVICE_DRIVER_INSTALL` System Restore point before any install |

> **Why both Windows Update *and* the catalog?** The Windows Update Agent only offers
> drivers Microsoft has promoted for your specific machine — it routinely reports "0
> updates" even when newer drivers exist. The Microsoft Update Catalog is the full online
> index, so the catalog scan is the real "find everything available online" feature.

## Architecture

The Rust backend **shells out to PowerShell** for the Windows-specific work
(driver enumeration, the Windows Update Agent, System Restore, vendor detection)
rather than binding the COM/Win32 APIs directly. This keeps the backend simple,
dependency-light (no `windows-rs`), and robust — the commands are well-trodden
and verified to work on the target machine.

```
src/                     Frontend (vanilla TS + Vite)
  index.html             UI shell
  main.ts                IPC calls + rendering
  styles.css             Dark UI theme
src-tauri/src/
  lib.rs                 Tauri builder + #[tauri::command] surface
  ps.rs                  PowerShell runner (incl. elevated helper) + JSON parsing
  drivers.rs             Installed-driver enumeration
  wupdate.rs             Windows Update scan + install
  vendor.rs              NVIDIA GPU detection
  safety.rs              System Restore point
```

### Commands (Rust → frontend)
- `scan_drivers()` → `DriverInfo[]`
- `check_online_updates()` → `OnlineUpdate[]` *(Microsoft Update Catalog scan, ~10s)*
- `install_catalog_update(updateId, createRestore)` → `InstallResult` *(elevated: download → extract → pnputil)*
- `check_windows_update()` → `UpdateInfo[]`
- `scan_vendors()` → `VendorDriverStatus[]`
- `install_update(updateId)` → `InstallResult` *(elevated)*
- `create_restore_point()` → `RestorePointResult` *(elevated)*

Catalog install ([`catalog_install.ps1`](src-tauri/scripts/catalog_install.ps1)) runs **elevated in
one UAC prompt**: optional restore point → resolve the package URL via the catalog DownloadDialog →
download the `.cab`/`.msu` → `expand` → `pnputil /add-driver /install` (handles reboot-required exit 3010).

The online catalog scan logic lives in [`src-tauri/scripts/catalog_scan.ps1`](src-tauri/scripts/catalog_scan.ps1)
(embedded into the binary via `include_str!`) — it dedupes hardware IDs and queries the
catalog concurrently through a PowerShell runspace pool (~8–10s for a full machine).

## Running it

Prerequisites: **Node**, **Rust (stable-msvc)**, and the **MSVC C++ Build Tools**.

```bash
npm install
npm run tauri dev      # dev mode with hot reload
npm run tauri build    # produce an NSIS installer (per-user, no admin to install)
```

## Elevation & safety model

- The app runs at normal integrity (`asInvoker`). Only the **install** and
  **restore-point** operations elevate, via `Start-Process -Verb RunAs` — so the
  WebView/JS never runs with admin rights.
- A System Restore point is created before any install (toggle in the header).
  System Protection must be enabled on `C:` for this to succeed; failures are
  surfaced and the user can choose to proceed without one.
- Cancelling the UAC prompt is handled gracefully (reported as "elevation declined").

## Notes & limitations

- **Windows Update** often lags behind GPU vendors — that's why the vendor check
  exists. The NVIDIA *latest-version* auto-lookup is intentionally **not**
  automated: NVIDIA's lookup endpoints are undocumented and their ToS prohibits
  automated access, so the app detects your current version and links you to the
  official download instead.
- Vendor coverage currently = NVIDIA detection. OEM tools (Dell `dcu-cli`,
  Lenovo Thin Installer, HP HPIA) are the natural next integrations.
- A future optimization is replacing the PowerShell shell-out with direct
  `windows-rs` bindings for speed.
