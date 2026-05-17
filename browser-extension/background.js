/* eslint-disable no-console */
/* global chrome */
/**
 * MV3 service worker — connects to Jadon's Excuses WebSocket bridge.
 * Protocol must match src-tauri/src/tracker/browser_bridge.rs TabEvent:
 * { type: "tab_event", url?, title?, domain?, app_name? }
 */
console.log("[bg] started");

const WS_URL = "ws://127.0.0.1:9876";
const APP_NAME = "Google Chrome";
const PAUSE_KEY = "jadons_pause_tracking";
const STORAGE_KEY = "jadons_tab_stats_v1";
/** Written for the extension popup connection indicator. */
const BRIDGE_CONNECTED_KEY = "jadons_bridge_ws_connected";

/** ~24s — Chrome alarms: use fractional delayInMinutes (periodInMinutes repeating min is 1). */
const KEEPALIVE_DELAY_MIN = 24 / 60;
const ALARM_KEEPALIVE = "jadons_keepalive";

/** Reconnect backoff: 1s, 2s, 4s … capped at 30s */
let reconnectAttempt = 0;
let reconnectTimer = null;
let socket = null;

function backoffMs() {
  const ms = Math.min(30_000, 1000 * 2 ** reconnectAttempt);
  reconnectAttempt += 1;
  return ms;
}

function resetBackoff() {
  reconnectAttempt = 0;
}

async function setBridgeConnected(on) {
  try {
    await chrome.storage.local.set({ [BRIDGE_CONNECTED_KEY]: on });
  } catch (_) {
    /* ignore */
  }
}

function clearReconnectTimer() {
  if (reconnectTimer != null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
}

function scheduleReconnect() {
  if (reconnectTimer != null) return;
  const ms = backoffMs();
  console.log("[bg] reconnect scheduled in", ms, "ms");
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectSocket();
  }, ms);
}

function connectSocket() {
  clearReconnectTimer();
  if (
    socket &&
    (socket.readyState === WebSocket.OPEN ||
      socket.readyState === WebSocket.CONNECTING)
  ) {
    return;
  }
  try {
    socket?.close();
  } catch (_) {
    /* ignore */
  }
  socket = null;
  void setBridgeConnected(false);
  console.log("[bg] connecting", WS_URL);
  try {
    const ws = new WebSocket(WS_URL);
    socket = ws;
    ws.addEventListener("open", () => {
      console.log("[bg] connected");
      resetBackoff();
      void setBridgeConnected(true);
    });
    ws.addEventListener("close", (ev) => {
      console.log("[bg] disconnected", ev.code, ev.reason || "");
      socket = null;
      void setBridgeConnected(false);
      scheduleReconnect();
    });
    ws.addEventListener("error", () => {
      console.log("[bg] ws error event");
      try {
        ws.close();
      } catch (_) {
        /* ignore */
      }
    });
  } catch (e) {
    console.log("[bg] WebSocket construct failed", e);
    scheduleReconnect();
  }
}

function sendRaw(obj) {
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    console.log("[bg] skip send (socket not open)");
    return;
  }
  const json = JSON.stringify(obj);
  console.log("[bg] sending", json.length > 280 ? `${json.slice(0, 280)}…` : json);
  try {
    socket.send(json);
  } catch (e) {
    console.log("[bg] send failed", e);
  }
}

/** Block by URL.protocol so chrome:// pages cannot slip through as hostname-only domains. */
function blockedUrlProtocol(url) {
  if (!url || typeof url !== "string") return true;
  try {
    const u = new URL(url);
    const proto = u.protocol.replace(/:$/, "").toLowerCase();
    if (
      [
        "chrome",
        "chrome-extension",
        "devtools",
        "about",
        "edge",
        "brave",
        "file",
      ].includes(proto)
    ) {
      return true;
    }
  } catch {
    return true;
  }
  const lower = url.trim().toLowerCase();
  return (
    lower.startsWith("chrome://") ||
    lower.startsWith("chrome-extension://") ||
    lower.startsWith("devtools://") ||
    lower.startsWith("file://") ||
    lower.startsWith("about:") ||
    lower.startsWith("edge://") ||
    lower.startsWith("brave://")
  );
}

function shouldSkipUrl(url) {
  return blockedUrlProtocol(url);
}

/** Hostname only, no www., lowercase — matches bridge host_from_url / TabEvent.domain */
function domainFromUrl(url) {
  try {
    const hostname = new URL(url).hostname;
    return hostname.replace(/^www\./i, "").toLowerCase();
  } catch {
    return "";
  }
}

async function paused() {
  const v = await chrome.storage.local.get(PAUSE_KEY);
  return Boolean(v[PAUSE_KEY]);
}

async function emitTab(tabId) {
  if (await paused()) return;
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab?.url) return;
  if (shouldSkipUrl(tab.url)) {
    console.log("[bg] skip tab_event (blocked scheme)", tab.url.slice(0, 80));
    return;
  }
  const domain = domainFromUrl(tab.url);
  if (!domain) {
    console.log("[bg] skip tab_event (empty domain)", tab.url.slice(0, 80));
    return;
  }
  const payload = {
    type: "tab_event",
    url: tab.url,
    title: tab.title ?? "",
    domain,
    app_name: APP_NAME,
  };
  console.log("[bg] tab_event url=", tab.url.slice(0, 120), "domain=", domain);
  sendRaw(payload);
}

async function emitActiveTab() {
  const [tab] = await chrome.tabs.query({
    active: true,
    lastFocusedWindow: true,
  });
  if (tab?.id) await emitTab(tab.id);
}

function scheduleKeepalive() {
  chrome.alarms.create(ALARM_KEEPALIVE, {
    delayInMinutes: KEEPALIVE_DELAY_MIN,
  });
}

chrome.alarms.onAlarm.addListener(async (alarm) => {
  if (alarm.name !== ALARM_KEEPALIVE) return;
  scheduleKeepalive();
  connectSocket();
  if (!(await paused())) await emitActiveTab();
});

chrome.runtime.onInstalled.addListener(() => {
  console.log("[bg] onInstalled");
  connectSocket();
  scheduleKeepalive();
});

chrome.runtime.onStartup.addListener(() => {
  console.log("[bg] onStartup");
  connectSocket();
  scheduleKeepalive();
});

chrome.tabs.onActivated.addListener(async (activeInfo) => {
  console.log("[bg] onActivated tabId=", activeInfo.tabId);
  await emitTab(activeInfo.tabId);
});

chrome.tabs.onUpdated.addListener(async (tabId, changeInfo, tab) => {
  const urlChanged = Object.prototype.hasOwnProperty.call(changeInfo, "url");
  const titleChanged = Object.prototype.hasOwnProperty.call(changeInfo, "title");
  const completed = changeInfo.status === "complete";
  if (urlChanged || titleChanged || completed) {
    console.log("[bg] onUpdated tabId=", tabId, "keys=", Object.keys(changeInfo));
    await emitTab(tabId);
  }
});

chrome.windows.onFocusChanged.addListener(async (windowId) => {
  if (windowId === chrome.windows.WINDOW_ID_NONE) {
    console.log("[bg] focus lost (no window)");
    return;
  }
  const [tab] = await chrome.tabs.query({ active: true, windowId });
  if (tab?.id) {
    console.log("[bg] window focus tabId=", tab.id);
    await emitTab(tab.id);
  }
});

chrome.runtime.onMessage.addListener((msg) => {
  if (msg?.type === "content_stats") {
    chrome.storage.local.get(STORAGE_KEY).then((cur) => {
      const prev = cur[STORAGE_KEY] || {};
      const tabId = String(msg.tabId ?? "");
      const bucket = prev[tabId] || {
        url: msg.url,
        domain: msg.domain,
        time_active_seconds: 0,
        click_count: 0,
      };
      bucket.time_active_seconds += msg.delta_seconds ?? 0;
      bucket.click_count += msg.click_delta ?? 0;
      bucket.url = msg.url;
      bucket.domain = msg.domain;
      prev[tabId] = bucket;
      chrome.storage.local.set({ [STORAGE_KEY]: prev });
    });
  }
});

connectSocket();
scheduleKeepalive();
