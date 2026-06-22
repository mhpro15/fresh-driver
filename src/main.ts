import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

// ---- Types (snake_case to match serde output from Rust) --------------------

interface DriverInfo {
  device_name: string;
  manufacturer: string | null;
  device_class: string | null;
  driver_version: string | null;
  driver_date: string | null;
  hardware_id: string | null;
  device_id: string;
  inf_name: string | null;
  vendor_name: string | null;
  chip_name: string | null;
}

interface UpdateInfo {
  update_id: string;
  title: string;
  driver_class: string | null;
  driver_model: string | null;
  driver_manufacturer: string | null;
  driver_ver_date: string | null;
  is_downloaded: boolean;
}

interface VendorDriverStatus {
  vendor: string;
  device_name: string;
  current_version: string | null;
  latest_version: string | null;
  download_url: string | null;
  status: string;
}

interface OnlineUpdate {
  query: string;
  device_name: string;
  device_class: string | null;
  installed_version: string | null;
  available_version: string | null;
  available_date: string | null;
  update_id: string | null;
  catalog_url: string | null;
  found: boolean;
}

interface InstallResult {
  success: boolean;
  reboot_required: boolean;
  message: string;
  published_names: string[];
}

interface HistoryEntry {
  device_name: string;
  version: string | null;
  published_names: string[];
  installed_at_unix: number;
}

interface RestorePointResult {
  success: boolean;
  message: string;
}

interface InstallProgress {
  stage: string;
  percent: number;
  message: string;
}

interface OemUpdate {
  name: string;
  category: string | null;
  available_version: string | null;
  current_version: string | null;
  download_url: string | null;
}

interface OemResult {
  brand: string | null;
  note: string | null;
  support_url: string | null;
  updates: OemUpdate[];
}

// ---- Small DOM helpers -----------------------------------------------------

const $ = <T extends HTMLElement = HTMLElement>(sel: string): T =>
  document.querySelector(sel) as T;

function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function toast(message: string, kind: "ok" | "err" = "ok") {
  const el = $("#toast");
  el.textContent = message;
  el.className = `toast ${kind}`;
  setTimeout(() => el.classList.add("hidden"), 4500);
}

function setGlobalStatus(message: string | null, kind: "info" | "err" = "info") {
  const el = $("#global-status");
  if (!message) {
    el.classList.add("hidden");
    return;
  }
  el.textContent = message;
  el.className = `global-status ${kind}`;
}

/// In-app confirmation dialog (replaces the browser confirm(), which shows the
/// page origin like "localhost"). Resolves true on confirm, false otherwise.
function confirmModal(
  title: string,
  body: string,
  okLabel = "Install",
  cancelLabel = "Cancel"
): Promise<boolean> {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.className = "modal-overlay";
    overlay.innerHTML = `
      <div class="modal">
        <div class="modal-title">${esc(title)}</div>
        <div class="modal-body">${esc(body).replace(/\n/g, "<br>")}</div>
        <div class="modal-actions">
          <button class="btn modal-cancel">${esc(cancelLabel)}</button>
          <button class="btn btn-primary modal-ok">${esc(okLabel)}</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);
    let done = false;
    const close = (v: boolean) => {
      if (done) return;
      done = true;
      overlay.remove();
      document.removeEventListener("keydown", onKey);
      resolve(v);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close(false);
      if (e.key === "Enter") close(true);
    };
    overlay.querySelector(".modal-ok")!.addEventListener("click", () => close(true));
    overlay.querySelector(".modal-cancel")!.addEventListener("click", () => close(false));
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) close(false);
    });
    document.addEventListener("keydown", onKey);
  });
}

// ---- State -----------------------------------------------------------------

let allDrivers: DriverInfo[] = [];
let availableUpdates: UpdateInfo[] = [];
let availableOnline: OnlineUpdate[] = [];
let installHistory: HistoryEntry[] = [];
let gpuCount = 0;

// ---- Rendering -------------------------------------------------------------

function animateCount(el: HTMLElement, target: number) {
  const from = parseInt(el.textContent || "0", 10) || 0;
  if (from === target) {
    el.textContent = String(target);
    return;
  }
  const start = performance.now();
  const dur = 500;
  const tick = (now: number) => {
    const p = Math.min(1, (now - start) / dur);
    const eased = 1 - Math.pow(1 - p, 3);
    el.textContent = String(Math.round(from + (target - from) * eased));
    if (p < 1) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

function renderSummary() {
  animateCount($("#stat-drivers .stat-num"), allDrivers.length);
  const total = availableOnline.length + availableUpdates.length;
  const updEl = $("#stat-updates");
  animateCount(updEl.querySelector(".stat-num")!, total);
  updEl.classList.toggle("has-updates", total > 0);
  animateCount($("#stat-gpu .stat-num"), gpuCount);

  // Sidebar badges
  const badge = $("#nav-updates-badge");
  badge.textContent = String(total);
  badge.classList.toggle("hidden", total === 0);
  $("#nav-drivers-count").textContent = allDrivers.length ? String(allDrivers.length) : "";
}

function renderOnline() {
  const list = $("#online-list");
  $("#online-meta").textContent = availableOnline.length
    ? `${availableOnline.length} newer driver(s) found online`
    : "";
  if (availableOnline.length === 0) {
    list.innerHTML = `<div class="placeholder ok">✓ No newer drivers found in the Microsoft Update Catalog for your hardware.</div>`;
    return;
  }
  list.innerHTML = availableOnline
    .map(
      (u) => `
      <div class="row online">
        <div class="row-main">
          <div class="row-title">${esc(u.device_name)}</div>
          <div class="row-sub">
            ${u.device_class ? `<span class="tag">${esc(u.device_class)}</span>` : ""}
            <span>Installed <strong class="mono">${esc(u.installed_version ?? "?")}</strong></span>
            <span class="arrow">→</span>
            <span>Available <strong class="mono new">${esc(u.available_version ?? "?")}</strong></span>
            ${u.available_date ? `<span class="muted">· ${esc(u.available_date)}</span>` : ""}
          </div>
        </div>
        <div class="row-actions">
          ${
            u.update_id
              ? `<button class="btn btn-install" data-id="${esc(u.update_id)}">Install</button>`
              : ""
          }
          ${
            u.catalog_url
              ? `<button class="btn btn-link" data-url="${esc(u.catalog_url)}">Catalog ↗</button>`
              : ""
          }
        </div>
      </div>`
    )
    .join("");

  list.querySelectorAll<HTMLButtonElement>(".btn-link").forEach((btn) =>
    btn.addEventListener("click", () => openUrl(btn.dataset.url!).catch(() => {}))
  );
  list.querySelectorAll<HTMLButtonElement>(".btn-install").forEach((btn) =>
    btn.addEventListener("click", () => installOnlineUpdate(btn.dataset.id!, btn))
  );
}

function renderOem(oem: OemResult) {
  const panel = $("#oem-panel");
  if (!oem.brand) {
    panel.classList.add("hidden");
    return;
  }
  panel.classList.remove("hidden");
  $("#oem-title").textContent = `${oem.brand} updates`;
  const support = $<HTMLButtonElement>("#oem-support");
  if (oem.support_url) {
    support.classList.remove("hidden");
    support.onclick = () => openUrl(oem.support_url!).catch(() => {});
  } else {
    support.classList.add("hidden");
  }

  const list = $("#oem-list");
  if (oem.updates.length === 0) {
    list.innerHTML = `<div class="placeholder">${esc(oem.note ?? `No ${oem.brand} updates found.`)}</div>`;
    return;
  }
  list.innerHTML = oem.updates
    .map(
      (u) => `
      <div class="row oem">
        <div class="row-main">
          <div class="row-title">${esc(u.name)}</div>
          <div class="row-sub">
            ${u.category ? `<span class="tag">${esc(u.category)}</span>` : ""}
            ${u.available_version ? `<span>${esc(u.available_version)}</span>` : ""}
          </div>
        </div>
        ${u.download_url ? `<button class="btn btn-install" data-url="${esc(u.download_url)}">Get</button>` : ""}
      </div>`
    )
    .join("");
  list.querySelectorAll<HTMLButtonElement>(".btn-install").forEach((btn) =>
    btn.addEventListener("click", () => openUrl(btn.dataset.url!).catch(() => {}))
  );
  if (oem.note) {
    list.insertAdjacentHTML("beforeend", `<div class="placeholder">${esc(oem.note)}</div>`);
  }
}

async function installOnlineUpdate(updateId: string, btn: HTMLButtonElement) {
  const wantRestore = $<HTMLInputElement>("#restore-toggle").checked;
  const proceed = await confirmModal(
    "Install this driver?",
    "This downloads and installs the driver from the Microsoft Update Catalog.\n\n" +
      "Windows will ask for administrator approval" +
      (wantRestore ? ", and a System Restore point will be created first." : ".")
  );
  if (!proceed) return;

  // Swap the row's action buttons for a live progress bar.
  const row = btn.closest(".row") as HTMLElement;
  const actions = row.querySelector(".row-actions") as HTMLElement;
  actions.innerHTML = `
    <div class="progress-wrap">
      <div class="progress-track"><div class="progress-fill"></div></div>
      <div class="progress-label">Requesting permission…</div>
    </div>`;
  const fill = actions.querySelector(".progress-fill") as HTMLElement;
  const label = actions.querySelector(".progress-label") as HTMLElement;

  // Poll backend progress until the install resolves.
  let polling = true;
  (async () => {
    while (polling) {
      try {
        const p = await invoke<InstallProgress>("get_install_progress");
        if (p && typeof p.percent === "number") {
          fill.style.width = `${Math.max(0, Math.min(100, p.percent))}%`;
          if (p.message) label.textContent = p.message;
        }
      } catch {
        /* transient file read — ignore */
      }
      await new Promise((r) => setTimeout(r, 350));
    }
  })();

  try {
    const target = availableOnline.find((u) => u.update_id === updateId);
    const res = await invoke<InstallResult>("install_catalog_update", {
      updateId,
      createRestore: wantRestore,
      deviceName: target?.device_name ?? "Driver",
      version: target?.available_version ?? "",
    });
    polling = false;
    if (res.success) {
      fill.style.width = "100%";
      label.textContent = res.reboot_required ? "Installed — reboot required" : "Installed ✓";
      toast(
        res.reboot_required
          ? "Installed — a reboot is required to finish."
          : "Driver installed successfully.",
        "ok"
      );
      window.setTimeout(() => {
        availableOnline = availableOnline.filter((u) => u.update_id !== updateId);
        renderOnline();
        renderSummary();
      }, 1300);
      loadHistory(); // surface the new roll-back entry
      invoke<DriverInfo[]>("scan_drivers")
        .then((d) => {
          allDrivers = d;
          renderDrivers($<HTMLInputElement>("#driver-search").value);
        })
        .catch(() => {});
    } else {
      toast(res.message, "err");
      renderOnline(); // restore the Install button
    }
  } catch (e) {
    polling = false;
    const msg = String(e);
    if (msg.toLowerCase().includes("cancel")) {
      toast("Installation cancelled.");
    } else {
      toast(`Install failed: ${msg}`, "err");
    }
    renderOnline();
  }
}

function renderUpdates() {
  const list = $("#updates-list");
  $("#updates-meta").textContent = availableUpdates.length
    ? `${availableUpdates.length} update(s) found`
    : "";
  if (availableUpdates.length === 0) {
    list.innerHTML = `<div class="placeholder ok">✓ No driver updates available from Windows Update — you're current.</div>`;
    return;
  }
  list.innerHTML = availableUpdates
    .map(
      (u) => `
      <div class="row update" data-id="${esc(u.update_id)}">
        <div class="row-main">
          <div class="row-title">${esc(u.title)}</div>
          <div class="row-sub">
            ${u.driver_class ? `<span class="tag">${esc(u.driver_class)}</span>` : ""}
            ${u.driver_manufacturer ? `<span>${esc(u.driver_manufacturer)}</span>` : ""}
            ${u.driver_ver_date ? `<span>· ${esc(u.driver_ver_date)}</span>` : ""}
          </div>
        </div>
        <button class="btn btn-install" data-id="${esc(u.update_id)}">Install</button>
      </div>`
    )
    .join("");

  list.querySelectorAll<HTMLButtonElement>(".btn-install").forEach((btn) =>
    btn.addEventListener("click", () => installUpdate(btn.dataset.id!, btn))
  );
}

function renderVendors(vendors: VendorDriverStatus[]) {
  const list = $("#vendor-list");
  $("#vendor-meta").textContent = vendors.length ? "" : "No supported GPU found";
  if (vendors.length === 0) {
    list.innerHTML = `<div class="placeholder">No NVIDIA GPU detected (vendor checks currently cover NVIDIA).</div>`;
    return;
  }
  list.innerHTML = vendors
    .map((v) => {
      const updateAvailable =
        v.latest_version && v.current_version && v.latest_version !== v.current_version;
      return `
      <div class="row vendor">
        <div class="row-main">
          <div class="row-title">${esc(v.device_name)} <span class="tag">${esc(v.vendor)}</span></div>
          <div class="row-sub">
            <span>Current: <strong>${esc(v.current_version ?? "unknown")}</strong></span>
            ${v.latest_version ? `<span>· Latest: <strong>${esc(v.latest_version)}</strong></span>` : ""}
            <span class="muted">· ${esc(v.status)}</span>
          </div>
        </div>
        ${
          v.download_url
            ? `<button class="btn btn-link" data-url="${esc(v.download_url)}">${updateAvailable ? "Get update" : "Open vendor"}</button>`
            : ""
        }
      </div>`;
    })
    .join("");

  list.querySelectorAll<HTMLButtonElement>(".btn-link").forEach((btn) =>
    btn.addEventListener("click", () => openUrl(btn.dataset.url!).catch(() => {}))
  );
}

function renderDrivers(filter = "") {
  const body = $("#drivers-body");
  const f = filter.trim().toLowerCase();
  const rows = (f
    ? allDrivers.filter((d) =>
        [d.device_name, d.device_class, d.manufacturer, d.driver_version, d.vendor_name, d.chip_name]
          .filter(Boolean)
          .some((s) => s!.toLowerCase().includes(f))
      )
    : allDrivers
  );
  if (rows.length === 0) {
    body.innerHTML = `<tr><td colspan="5" class="placeholder">No matching drivers.</td></tr>`;
    return;
  }
  body.innerHTML = rows
    .map(
      (d) => {
        const showChip = d.chip_name && d.chip_name !== d.device_name;
        return `
      <tr>
        <td>${esc(d.device_class ?? "")}</td>
        <td>
          <div>${esc(d.device_name)}</div>
          ${showChip ? `<div class="cell-sub">${esc(d.chip_name)}</div>` : ""}
        </td>
        <td class="mono">${esc(d.driver_version ?? "")}</td>
        <td class="mono">${esc(d.driver_date ?? "")}</td>
        <td>${esc(d.vendor_name ?? d.manufacturer ?? "")}</td>
      </tr>`;
      }
    )
    .join("");
}

function timeAgo(unix: number): string {
  if (!unix) return "";
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

function renderHistory() {
  const list = $("#history-list");
  $("#nav-history-dot").classList.toggle("hidden", installHistory.length === 0);
  if (installHistory.length === 0) {
    list.innerHTML = `<div class="placeholder">No drivers installed through Fresh Driver yet. Installed drivers will appear here so you can roll them back.</div>`;
    return;
  }
  list.innerHTML = installHistory
    .map(
      (h) => `
      <div class="row history">
        <div class="row-main">
          <div class="row-title">${esc(h.device_name)}</div>
          <div class="row-sub">
            ${h.version ? `<span>v<strong class="mono">${esc(h.version)}</strong></span>` : ""}
            <span class="tag">${esc(h.published_names.join(", "))}</span>
            <span class="muted">· ${esc(timeAgo(h.installed_at_unix))}</span>
          </div>
        </div>
        <div class="row-actions">
          <button class="btn btn-rollback" data-name="${esc(h.published_names[0] ?? "")}">↶ Roll back</button>
        </div>
      </div>`
    )
    .join("");
  list.querySelectorAll<HTMLButtonElement>(".btn-rollback").forEach((btn) =>
    btn.addEventListener("click", () => rollbackDriver(btn.dataset.name!, btn))
  );
}

async function loadHistory() {
  try {
    installHistory = await invoke<HistoryEntry[]>("get_install_history");
  } catch {
    installHistory = [];
  }
  renderHistory();
}

async function rollbackDriver(publishedName: string, btn: HTMLButtonElement) {
  if (!publishedName) return;
  const proceed = await confirmModal(
    "Roll back this driver?",
    "This removes the driver you installed and reverts the device to its previous driver.\n\n" +
      "Windows will ask for administrator approval.",
    "Roll back"
  );
  if (!proceed) return;
  btn.disabled = true;
  btn.textContent = "Rolling back…";
  try {
    const res = await invoke<InstallResult>("rollback_driver", { publishedName });
    if (res.success) {
      toast(
        res.reboot_required ? "Rolled back — reboot to finish." : "Driver rolled back.",
        "ok"
      );
      await loadHistory();
      invoke<DriverInfo[]>("scan_drivers")
        .then((d) => {
          allDrivers = d;
          renderDrivers($<HTMLInputElement>("#driver-search").value);
        })
        .catch(() => {});
    } else {
      toast(res.message, "err");
      btn.disabled = false;
      btn.textContent = "↶ Roll back";
    }
  } catch (e) {
    const msg = String(e);
    toast(msg.toLowerCase().includes("cancel") ? "Rollback cancelled." : `Rollback failed: ${msg}`);
    btn.disabled = false;
    btn.textContent = "↶ Roll back";
  }
}

// ---- Actions ---------------------------------------------------------------

async function refreshAll() {
  const btn = $<HTMLButtonElement>("#refresh-btn");
  btn.disabled = true;
  btn.classList.add("scanning");
  $("#refresh-btn .refresh-text").textContent = "Scanning…";
  setGlobalStatus("Scanning drivers, Windows Update and the online catalog…");

  loadHistory(); // fast, local — show roll-back options immediately

  // Each scan renders as it resolves. Failures degrade quietly with a calm
  // in-panel note — never a raw error toast (logged to console for debugging).
  const driversP = invoke<DriverInfo[]>("scan_drivers")
    .then((d) => {
      allDrivers = d;
      renderDrivers($<HTMLInputElement>("#driver-search").value);
      renderSummary();
    })
    .catch((e) => {
      console.error("driver scan:", e);
      $("#drivers-body").innerHTML = `<tr><td colspan="5" class="placeholder">Couldn't read installed drivers right now.</td></tr>`;
    });

  const vendorsP = invoke<VendorDriverStatus[]>("scan_vendors")
    .then((v) => {
      renderVendors(v);
      gpuCount = v.length;
      renderSummary();
    })
    .catch((e) => {
      console.error("vendor scan:", e);
      $("#vendor-list").innerHTML = `<div class="placeholder">No graphics info available.</div>`;
    });

  const updatesP = invoke<UpdateInfo[]>("check_windows_update")
    .then((u) => {
      availableUpdates = u;
      renderUpdates();
      renderSummary();
    })
    .catch((e) => {
      console.error("windows update scan:", e);
      $("#updates-list").innerHTML = `<div class="placeholder">Couldn't check Windows Update right now.</div>`;
    });

  // The online catalog scan is the comprehensive check (~10s).
  $("#online-list").innerHTML = `<div class="placeholder">Scanning Microsoft Update Catalog online…</div>`;
  const onlineP = invoke<OnlineUpdate[]>("check_online_updates")
    .then((u) => {
      availableOnline = u;
      renderOnline();
      renderSummary();
    })
    .catch((e) => {
      console.error("online catalog scan:", e);
      $("#online-list").innerHTML = `<div class="placeholder">Couldn't reach the online catalog right now.</div>`;
    });

  // Manufacturer (Dell/HP/Lenovo) updates — only surfaces on those brands.
  const oemP = invoke<OemResult>("scan_oem")
    .then((o) => renderOem(o))
    .catch(() => {}); // non-fatal; panel stays hidden

  await Promise.all([driversP, vendorsP, updatesP, onlineP, oemP]);
  renderSummary();

  setGlobalStatus(null);
  btn.disabled = false;
  btn.classList.remove("scanning");
  $("#refresh-btn .refresh-text").textContent = "Refresh";
}

async function installUpdate(updateId: string, btn: HTMLButtonElement) {
  const wantRestore = $<HTMLInputElement>("#restore-toggle").checked;
  btn.disabled = true;

  try {
    if (wantRestore) {
      btn.textContent = "Restore point…";
      const rp = await invoke<RestorePointResult>("create_restore_point");
      if (!rp.success) {
        const proceed = await confirmModal(
          "Restore point failed",
          `${rp.message}\n\nContinue installing without a restore point?`,
          "Continue"
        );
        if (!proceed) {
          btn.disabled = false;
          btn.textContent = "Install";
          return;
        }
      } else {
        toast("Restore point created.", "ok");
      }
    }

    btn.textContent = "Installing…";
    const res = await invoke<InstallResult>("install_update", { updateId });
    if (res.success) {
      toast(
        res.reboot_required
          ? "Installed — a reboot is required to finish."
          : "Driver installed successfully.",
        "ok"
      );
      // Drop it from the list and refresh counts.
      availableUpdates = availableUpdates.filter((u) => u.update_id !== updateId);
      renderUpdates();
      renderSummary();
      // Re-scan drivers so the table reflects the new version.
      invoke<DriverInfo[]>("scan_drivers")
        .then((d) => {
          allDrivers = d;
          renderDrivers($<HTMLInputElement>("#driver-search").value);
        })
        .catch(() => {});
    } else {
      toast(res.message, "err");
      btn.disabled = false;
      btn.textContent = "Retry";
    }
  } catch (e) {
    toast(`Install failed: ${e}`, "err");
    btn.disabled = false;
    btn.textContent = "Retry";
  }
}

// ---- Theme & navigation ----------------------------------------------------

function applyTheme(theme: "light" | "dark") {
  document.body.setAttribute("data-theme", theme);
  localStorage.setItem("fd-theme", theme);
  const label = document.querySelector("#theme-toggle .theme-label");
  if (label) label.textContent = theme === "dark" ? "Dark" : "Light";
}

function initTheme() {
  const saved = localStorage.getItem("fd-theme");
  applyTheme(saved === "dark" ? "dark" : "light"); // light by default
}

function toggleTheme() {
  const current = document.body.getAttribute("data-theme") === "dark" ? "dark" : "light";
  applyTheme(current === "dark" ? "light" : "dark");
}

const VIEW_TITLES: Record<string, string> = {
  overview: "Overview",
  updates: "Updates",
  drivers: "All drivers",
  history: "History",
};

function setView(view: string) {
  document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
  document.querySelector(`#view-${view}`)?.classList.add("active");
  document.querySelectorAll<HTMLElement>(".nav-item").forEach((n) =>
    n.classList.toggle("active", n.dataset.view === view)
  );
  $("#page-title").textContent = VIEW_TITLES[view] ?? "Overview";
}

// ---- Wire up ---------------------------------------------------------------

window.addEventListener("DOMContentLoaded", () => {
  initTheme();
  $("#theme-toggle").addEventListener("click", toggleTheme);

  document.querySelectorAll<HTMLElement>(".nav-item").forEach((n) =>
    n.addEventListener("click", () => setView(n.dataset.view!))
  );
  // Summary stat cards jump to their related view.
  document.querySelectorAll<HTMLElement>("[data-go]").forEach((el) =>
    el.addEventListener("click", () => setView(el.dataset.go!))
  );

  $("#refresh-btn").addEventListener("click", refreshAll);
  $<HTMLInputElement>("#driver-search").addEventListener("input", (e) =>
    renderDrivers((e.target as HTMLInputElement).value)
  );

  // Auto-scan on launch.
  refreshAll();
});
