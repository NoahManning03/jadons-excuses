# Jadon's Excuses — Browser Tab Tracker (dev)

This Manifest V3 extension sends **only tab metadata** (URL, domain, title) to the
local desktop app over `ws://127.0.0.1:9876`. It does **not** read page HTML and
does not ship data to the internet.

## Install (Chrome / Chromium)

Step-by-step instructions for sharing with someone else: **[INSTALL.md](./INSTALL.md)**.

Quick dev install:

1. Build & run the desktop app (`pnpm tauri dev`) so the WebSocket listener is up.
2. Open `chrome://extensions`.
3. Enable **Developer mode**.
4. Click **Load unpacked** and select this `browser-extension/` folder.

## Privacy

- No remote servers.
- No page-content scraping — only `tabs` API metadata plus a lightweight
  on-page counter in `content-script.js` (clicks / time / scroll depth) stored
  locally in `chrome.storage.local` for the popup chart.

## Notes

- Chrome alarms may not fire faster than ~1 minute in some builds; the
  background worker still updates on tab switches and navigation completions.
- **Popup → desktop app:** the primary button opens `jadons-excuses://dashboard`,
  which macOS routes to the installed **Jadon's Excuses** app (URL scheme
  registered in the desktop bundle). There is no dependency on a local Vite
  dev server for end users.
- The popup polls `chrome.storage.local` every few seconds for
  `jadons_bridge_ws_connected` (maintained by `background.js` from the WebSocket
  state) so the connection line stays accurate while the popup is open.
