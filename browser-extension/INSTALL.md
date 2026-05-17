# Install Jadon’s Excuses — Tab Tracker (unpacked)

Use these steps to install the extension **without** the Chrome Web Store. You only need Google Chrome on your computer.

## Before you start

1. **Install the Jadon’s Excuses desktop app** on the same machine and leave it running so it can receive tab updates (the extension talks to the app on your computer only).
2. **Use this folder** — the one that contains `manifest.json`, `background.js`, and the `icons` folder. Do not move files out of that folder after installation.

## Install in Chrome

1. Open **Google Chrome**.
2. Copy this address, paste it into the address bar, and press **Enter**:
   ```text
   chrome://extensions
   ```
3. Turn **Developer mode** **ON** (toggle in the top-right of the page).
4. Click **Load unpacked**.
5. Choose the **`browser-extension`** folder from this project (the folder that contains `manifest.json`).
6. You should see **“Jadon’s Excuses — Tab Tracker”** in your extensions list with the orange **JE** icon.

## After installing

- **Pin the extension** (puzzle icon → pin) if you want it always visible.
- If Chrome says the extension needs permission for sites, allow it when prompted — the extension only sends activity to your **local** desktop app.
- If you update the files (new version from a friend), go back to `chrome://extensions` and click **Reload** on this extension.

## Troubleshooting

| Issue | What to try |
|--------|-------------|
| “Manifest file is missing or unreadable” | Make sure you selected the **`browser-extension`** folder itself, not a parent folder. |
| Extension won’t connect | Open the Jadon’s Excuses app first; then reload the extension. |
| Errors after updating the extension | Close old tabs or reload them once; or restart Chrome. |

## Privacy

Nothing is sent to the cloud — communication stays between Chrome on your machine and the Jadon’s Excuses app.

---

## Sharing this folder with a friend (no Web Store)

1. Zip the entire **`browser-extension`** folder (include `manifest.json`, `icons`, `background.js`, etc.).
2. Send the zip (AirDrop, email, cloud drive — your choice).
3. Your friend unzips it, then follows **Install in Chrome** above, choosing the **unzipped** folder when Chrome asks for the extension directory.

Keep the folder structure unchanged — Chrome loads from one directory that must contain `manifest.json` at the top level.

### Chrome Web Store (later)

When you publish to the store, use **`store-assets/icon-marketing.png`** (128×128 source) for listing artwork. You still ship the same unpacked folder structure inside the `.zip` you upload to the developer dashboard.
