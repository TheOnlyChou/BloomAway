# Milo Firefox extension

This development WebExtension classifies only the active Firefox tab as
`YouTube`, `YouTubeShorts`, `Instagram`, `InstagramReels`, or `Other`. Category
transitions are logged in the background console and sent to Milo through
Firefox Native Messaging. Repeated categories remain deduplicated.

## Permissions

The Manifest V3 extension requests `tabs` so its background script can read the
active `Tab.url`, plus `nativeMessaging` so it can connect to
`com.milo.desktop`. It has no host permissions, content scripts, injected code,
network requests, or storage.

The stable Gecko extension ID is
`milo-browser-activity@bloomaway.local`. The native host manifest lists exactly
that ID in `allowed_extensions`; Firefox therefore exposes this host only to
the Milo extension.

## Build and install the development native host

From the repository root:

```bash
cargo build --bin milo-native-host
./extension/native-host/install.sh
```

The script resolves the binary to an absolute path and generates the manifest
at Arch Linux native Firefox's user-level location:

```text
~/.mozilla/native-messaging-hosts/com.milo.desktop.json
```

No root access is used. The committed JSON is a template with a
`__MILO_NATIVE_HOST_PATH__` placeholder. To select another built binary, pass
its path to the script. To remove the development registration, remove only
the generated manifest:

```bash
rm ~/.mozilla/native-messaging-hosts/com.milo.desktop.json
```

This does not edit any Firefox profile or extension file.

## Load temporarily in Firefox

1. Open `about:debugging` in Firefox.
2. Select **This Firefox**.
3. Select **Load Temporary Add-on**.
4. Choose `extension/manifest.json` from this repository.
5. Find **Milo Browser Activity** in the temporary extensions list and select
   **Inspect** to open its background console.

After changing an extension file or reinstalling the host, use **Reload** on
the extension's `about:debugging` entry. A temporary extension is removed when
Firefox exits.

## Event handling

The background script queries the active tab once when it starts. Ongoing
tracking is event-driven:

- `tabs.onActivated` selects a newly active tab.
- `tabs.onUpdated`, filtered to URL changes, observes navigation only when its
  tab ID matches the currently selected tab.
- `windows.onFocusChanged` updates the selected tab when focus moves between
  Firefox windows and clears it while Firefox has no focused window.

The selected tab and previous category exist only in memory. Background-tab
updates are ignored. Repeated events for the same category are deduplicated,
including different videos or pages that remain within one category.

The extension keeps one persistent
`runtime.connectNative("com.milo.desktop")` port. On disconnect it clears the
port; a later category transition makes one new connection attempt. There is
no retry timer, and a missing host or stopped Milo cannot interrupt browsing.

Only messages with this shape cross Native Messaging:

```json
{"type":"browser_activity","activity":"youtube_shorts"}
```

The other values are `youtube`, `instagram`, `instagram_reels`, and `other`.
Full URLs and page titles are never sent to the native host.

## URL classification

`classifyUrl(url)` uses the built-in `URL` parser and exact host matching.
Only HTTP and HTTPS pages on `youtube.com`, `www.youtube.com`, `instagram.com`,
or `www.instagram.com` receive site categories. YouTube paths beginning with
`/shorts/` become `YouTubeShorts`; Instagram paths beginning with `/reel/` or
`/reels/` become `InstagramReels`. Missing, malformed, internal, and all other
URLs become `Other`.

## Exact manual integration test

1. Build both executables and install the development host:

   ```bash
   cargo build
   ./extension/native-host/install.sh
   ```

2. Start Milo with `cargo run`.
3. Load or reload `extension/manifest.json` through `about:debugging`.
4. Open normal YouTube. Expected Milo output:

   ```text
   [milo] browser activity: YouTube
   ```

5. Open YouTube Shorts. Expected: `[milo] browser activity: YouTubeShorts`.
6. Open Instagram. Expected: `[milo] browser activity: Instagram`.
7. Open Reels. Expected: `[milo] browser activity: InstagramReels`.
8. Open another site to verify `Other` if desired.
9. Stop Milo and continue browsing. Firefox pages must continue working; the
   extension console may report a native-host disconnect.

## Classification test cases

With the background console open, activate tabs containing these URLs:

| URL | Expected category |
| --- | --- |
| `https://www.youtube.com/` | `YouTube` |
| `https://www.youtube.com/watch?v=test` | `YouTube` |
| `https://www.youtube.com/shorts/test` | `YouTubeShorts` |
| `https://www.instagram.com/` | `Instagram` |
| `https://www.instagram.com/reels/` | `InstagramReels` |
| `https://www.instagram.com/reel/example/` | `InstagramReels` |
| `https://example.com/` | `Other` |
| `about:blank` | `Other` |

Expected output appears only when the category changes:

```text
[milo-extension] YouTubeShorts
https://www.youtube.com/shorts/test
```

To verify filtering and deduplication:

1. Leave a YouTube tab active and navigate another background tab. No event
   should be printed for the background tab.
2. Move between two normal YouTube pages. No second `YouTube` event should be
   printed.
3. Move from a normal YouTube page to a Shorts URL. One `YouTubeShorts` event
   should be printed.
4. Move between two Shorts videos. No second `YouTubeShorts` event should be
   printed.

No browsing history is collected. URLs are not persisted or transmitted (they
appear only in the existing local debug log), and browser activity remains
ephemeral. The extension performs no scoring, timers, blocking, tab closing,
intervention, content inspection, scroll monitoring, dialogue, or state logic.
