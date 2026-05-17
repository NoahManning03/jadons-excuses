# Chrome Web Store — Jadon's Excuses Tab Tracker (submission notes)

## Developer registration

- One-time **$5 USD** developer registration fee for the Chrome Web Store.
- Pay and manage listings at **[Chrome Web Store Developer Dashboard](https://chrome.google.com/webstore/devconsole/)**.

## Listing assets checklist

| Asset | Requirement |
|--------|----------------|
| **Icons** | **16×16**, **48×48**, and **128×128** PNG (referenced in `manifest.json` under `icons`). |
| **Screenshots** | At least **1**; use **1280×800** or **640×400** pixels, **PNG** or **JPG**. |
| **Promotional tile** (optional but recommended) | **440×280** pixels. |
| **Short description** | Max **132 characters** (store-enforced). |

---

## Pre-written short description (≤132 characters)

Use this verbatim in the store listing:

```text
Tracks browsing domains for the Jadon's Excuses time-tracking desktop app. Local-only — nothing leaves your computer.
```

---

## Pre-written long description (markdown)

Paste or adapt the following as the **detailed description** in the developer dashboard (many fields accept markdown or plain text):

### What it does

**Jadon's Excuses — Tab Tracker** works with the **Jadon's Excuses** desktop time-tracking app on the **same computer**. It reports which site domains you visit so your local app can attribute time to projects — **nothing is uploaded to the cloud**.

### Local-only by design

- All communication stays on **your machine**.
- The extension sends activity to a **local WebSocket** endpoint (**127.0.0.1**) that only your desktop app listens on.
- **No** remote analytics, **no** telemetry, **no** third-party scripts added to pages.

### Requirements

- Install and run the **Jadon's Excuses desktop application** on this PC or Mac. Without it, the extension has nowhere to send data and will simply not connect — **no data leaves your device** either way.

### What data is involved

- **Tab / URL metadata** (e.g. domain, page title) as you browse, plus lightweight **on-page signals** (e.g. visibility and coarse interaction hints) used only for the local app’s dashboard — **not** full page content or keystrokes.

---

## Privacy policy (for `PRIVACY.md` or hosted page)

Save this as **PRIVACY.md** in your repo and host it on **GitHub Pages** or any static URL; paste that URL into the store’s privacy policy field:

```text
Jadon's Excuses — Tab Tracker does not collect, transmit, or store any data on remote servers. All data captured by this extension is sent exclusively to a local WebSocket endpoint (127.0.0.1:9876) for use by the Jadon's Excuses desktop application running on the same computer. The extension cannot communicate with any external network. No analytics, telemetry, or identifiers are sent anywhere. If the desktop app is not running, the extension silently fails to send data.
```

---

## How to zip and upload (Chrome Web Store)

The store expects a **`.zip`** whose **root** contains `manifest.json` (not a folder that then contains the extension).

### Include

- `manifest.json`
- `icons/` (all referenced icon files)
- `background.js`
- `content-script.js`
- `popup.html`, `popup.js`, `popup.css` (and any other files your manifest references)

### Exclude

- **`.DS_Store`** (macOS junk)
- **`.git`** — never zip the repo; zip only the extension folder contents
- **`node_modules`** — not used by this MV3 extension package
- **`INSTALL.md`** — for unpacked / friend-share installs only, not for store submission
- **`store-assets/`** — listing artwork and this doc; not required inside the shipped `.zip`

### Shell command (from repo root)

Creates `jadons-tab-tracker-store-1.0.0.zip` **next to** `browser-extension/`:

```bash
cd browser-extension && zip -r ../jadons-tab-tracker-store-1.0.0.zip . -x "*.DS_Store" -x "store-assets/*" -x "INSTALL.md"
```

Upload **`jadons-tab-tracker-store-1.0.0.zip`** in the [Developer Dashboard](https://chrome.google.com/webstore/devconsole/).

---

## After publishing — orphaned tabs (MV3)

If you reload the extension during development, already-open tabs may log **one** “extension context invalidated” error until those tabs are refreshed or closed. That is normal for Manifest V3; fresh tabs use the new extension context.
