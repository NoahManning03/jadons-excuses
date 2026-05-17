/* global chrome */
/**
 * Lightweight stats for storage only — does NOT drive the bridge (background does).
 * After an extension reload, orphaned tabs throw once on sendMessage, then go silent.
 */
(() => {
  if (window.__jadons_content_script_loaded__) return;
  window.__jadons_content_script_loaded__ = true;

  let invalidated = false;
  let intervalId = null;
  let clicks = 0;
  let maxScrollY = 0;

  function stopAll() {
    invalidated = true;
    if (intervalId != null) {
      try {
        clearInterval(intervalId);
      } catch (_) {}
      intervalId = null;
    }
  }

  function safeSend(payload) {
    if (invalidated) return;
    try {
      chrome.runtime.sendMessage(payload);
    } catch (_) {
      stopAll();
    }
  }

  document.addEventListener(
    "click",
    () => {
      if (invalidated) return;
      clicks += 1;
    },
    true,
  );

  window.addEventListener("scroll", () => {
    if (invalidated) return;
    maxScrollY = Math.max(maxScrollY, window.scrollY || 0);
  });

  document.addEventListener("visibilitychange", () => {
    if (invalidated) return;
    safeSend({
      type: "content_stats",
      delta_seconds: 0,
      click_delta: 0,
      url: location.href,
      domain: location.hostname.replace(/^www\./i, "").toLowerCase(),
      visibility: document.visibilityState,
      scroll_max_y: maxScrollY,
    });
  });

  intervalId = setInterval(() => {
    if (invalidated) return;
    const click_delta = clicks;
    clicks = 0;
    safeSend({
      type: "content_stats",
      delta_seconds: 10,
      click_delta,
      url: location.href,
      domain: location.hostname.replace(/^www\./i, "").toLowerCase(),
      scroll_max_y: maxScrollY,
    });
  }, 10_000);
})();
