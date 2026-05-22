const PAUSE_KEY = "jadons_pause_tracking";
const STORAGE_KEY = "jadons_tab_stats_v1";
const BRIDGE_CONNECTED_KEY = "jadons_bridge_ws_connected";

const DEEP_LINK = "jadons-excuses://dashboard";

function setDot(ok) {
  const el = document.getElementById("status-dot");
  if (!el) return;
  el.style.background = ok ? "#10B981" : "#94A3B8";
}

function getDownloadInfo() {
  const ua = navigator.userAgent;
  if (/Windows/i.test(ua)) {
    return {
      url: "https://github.com/NoahManning03/jadons-excuses/releases/download/v1.0.2/Jadon.s.Excuses_0.1.0_x64_en-US.msi",
      label: "⬇ Download for Windows — Free",
      meta: "Windows 10/11 · MSI installer"
    };
  }
  return {
    url: "https://github.com/NoahManning03/jadons-excuses/releases/download/v1.0.2/Jadon.s.Excuses_0.1.0_aarch64.dmg",
    label: "⬇ Download for macOS — Free",
    meta: "macOS · 3.8 MB · Notarized by Apple"
  };
}

function setConnectionStatus(connected) {
  const el = document.getElementById("connection-status");
  if (el) {
    if (connected) {
      el.textContent = "🟢 Connected to Jadon's Excuses";
      el.classList.remove("connection-status--warn");
      el.classList.add("connection-status--ok");
    } else {
      el.textContent = "🟡 Open the Jadon's Excuses app to start tracking";
      el.classList.remove("connection-status--ok");
      el.classList.add("connection-status--warn");
    }
  }

  const openApp = document.getElementById("open-app");
  const downloadCta = document.getElementById("download-cta");
  if (openApp) openApp.style.display = connected ? "block" : "none";
  if (downloadCta) downloadCta.style.display = connected ? "none" : "flex";
  if (!connected) {
    const info = getDownloadInfo();
    const link = document.querySelector(".btn-download");
    const meta = document.querySelector(".download-meta");
    if (link) { link.href = info.url; link.textContent = info.label; }
    if (meta) meta.textContent = info.meta;
  }
  document.body.classList.toggle("body--disconnected", !connected);
}

async function readBridgeConnected() {
  const v = await chrome.storage.local.get(BRIDGE_CONNECTED_KEY);
  return Boolean(v[BRIDGE_CONNECTED_KEY]);
}

async function render() {
  const connected = await readBridgeConnected();
  setConnectionStatus(connected);

  const { [STORAGE_KEY]: raw, [PAUSE_KEY]: paused } = await chrome.storage.local.get([
    STORAGE_KEY,
    PAUSE_KEY,
  ]);
  const stats = raw || {};
  const rows = Object.values(stats)
    .filter((x) => x && x.domain)
    .sort((a, b) => (b.time_active_seconds || 0) - (a.time_active_seconds || 0))
    .slice(0, 5);

  const host = document.getElementById("domains");
  if (!host) return;
  host.innerHTML = "";
  if (!rows.length) {
    host.textContent = "No domains yet — browse a bit first.";
    setDot(false);
    return;
  }
  setDot(connected);
  const max = Math.max(1, ...rows.map((r) => r.time_active_seconds || 0));
  for (const r of rows) {
    const row = document.createElement("div");
    row.className = "domain-row";
    const label = document.createElement("div");
    label.textContent = r.domain;
    const bar = document.createElement("div");
    bar.className = "bar";
    const inner = document.createElement("div");
    inner.className = "bar-inner";
    inner.style.width = `${Math.round(((r.time_active_seconds || 0) / max) * 100)}%`;
    bar.appendChild(inner);
    const meta = document.createElement("div");
    meta.className = "meta";
    meta.textContent = `${Math.round((r.time_active_seconds || 0) / 60)}m`;
    row.appendChild(label);
    row.appendChild(bar);
    row.appendChild(meta);
    host.appendChild(row);
  }

  const pause = document.getElementById("pause");
  if (pause) pause.checked = Boolean(paused);
}

document.getElementById("pause")?.addEventListener("change", async (e) => {
  const on = e.target.checked;
  await chrome.storage.local.set({ [PAUSE_KEY]: on });
});

document.getElementById("open-app")?.addEventListener("click", () => {
  chrome.tabs.create({ url: DEEP_LINK });
});

void render();

let pollId = window.setInterval(() => {
  void render();
}, 2000);

window.addEventListener("unload", () => {
  window.clearInterval(pollId);
});
