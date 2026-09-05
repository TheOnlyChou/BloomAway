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
./extension/native-host/install.sh
```

With no binary argument, the script first runs
`cargo build --bin milo-native-host`. This ensures the manifest does not keep
launching a stale helper after its source changes. The script then resolves the
binary to an absolute path and generates the manifest at Arch Linux native
Firefox's user-level location:

```text
~/.mozilla/native-messaging-hosts/com.milo.desktop.json
```

No root access is used. The committed JSON is a template with a
`__MILO_NATIVE_HOST_PATH__` placeholder. To select another built binary, pass
its path to the script; explicitly supplied binaries are not rebuilt. To remove
the development registration, remove only the generated manifest:

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
The module-level port is reused for every category transition and is cleared
only by its `onDisconnect` callback. The background console logs
`[milo-extension] native host connected` when the port is created and logs
`[milo-extension] native host disconnected` only when Firefox reports that
connection ending.

The helper independently retains one Unix stream to the running Milo process.
If that local stream cannot be opened or written, the helper reports the error
to stderr, keeps reading Firefox Native Messaging frames, and tries Milo again
on a later activity message. Local socket failure therefore does not tear down
the extension port. Firefox stdin EOF (or an unusable Native Messaging frame)
ends the helper and sends an explicit tracking-unavailable message to Milo when
the local connection is available.

Activity messages from Firefox have this shape:

```json
{"type":"browser_activity","activity":"youtube_shorts"}
```

The other values are `youtube`, `instagram`, `instagram_reels`, and `other`.
Full URLs and page titles are never sent to the native host.

The reverse direction accepts exactly one command:

```json
{"type":"browser_command","command":"close_active_distraction_tab"}
```

For every valid command, the extension freshly obtains Firefox's focused
window and queries its active tab. It classifies that tab's current URL with
`classifyUrl()` immediately before removal. Only a current `YouTubeShorts` or
`InstagramReels` tab is removed; it never searches background tabs or other
windows. Normal YouTube, Instagram, other or inaccessible pages are left
untouched. This fresh query is also the stale-command guard if the user changes
tabs after the popup appeared.

The extension returns `closed`, `ignored_not_distracting`, `no_active_tab`, or
`error` as a semantic result. It sends no URL, title, tab ID, or page content:

```json
{"type":"browser_command_result","command":"close_active_distraction_tab","result":"closed"}
```

Command diagnostics identify each boundary without logging the URL. A
successful action logs command receipt, active-tab query and classification,
the removal attempt, `tab closed`, and the semantic result. Invalid native
messages are reported without dumping their contents.

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
4. In the extension background console, expect this once:

   ```text
   [milo-extension] native host connected
   ```

5. Open normal YouTube. Expected Milo output:

   ```text
   [milo] browser activity: YouTube
   ```

6. Open YouTube Shorts. Expect
   `[milo] browser activity: YouTubeShorts`, followed by one continuous session:

   ```text
   [milo] distraction started: YouTubeShorts
   [milo] distraction 10s: YouTubeShorts
   [milo] distraction 20s: YouTubeShorts
   [milo] distraction 30s: YouTubeShorts
   [milo] intervention requested: StillScrolling
   [milo] intervention presentation: Show
   [milo] GTK: calling intervention popover.popup()
   ```

   There must be no immediate `native host disconnected` message. Milo should
   progress Idle -> Curious -> Concerned -> intervention popup over 35 seconds.
7. Open normal YouTube. The same native-host port remains connected, the
   browser category changes, and the distraction session ends normally.
8. Open Instagram. Expected: `[milo] browser activity: Instagram`.
9. Open Reels. Expected: `[milo] browser activity: InstagramReels`.
10. Stop Milo and continue browsing. Firefox pages and the native-host port
    must remain unaffected; a later category transition may produce only a
    local Milo-socket diagnostic from the helper.
11. Reload the extension or close Firefox. That genuine Native Messaging
    closure may produce `[milo-extension] native host disconnected`.

## Manual browser-command tests

- **Keep scrolling:** Reach the popup on Shorts and choose `Keep scrolling`.
  The popup closes, the tab remains open, Milo remains Concerned, and no
  browser command is sent.
- **Take a break:** In a fresh Shorts session, reach the popup and choose `Take
  a break`. The active Shorts tab closes. The extension then observes the new
  active tab normally; Milo does not force its state from the command result.
- **Stale command:** Reach the popup, switch to a normal site before the command
  is handled, then trigger `TakeBreak` if possible. The extension reclassifies
  the current tab as non-distracting and does not close it.
- **Instagram Reels:** Repeat the successful `Take a break` flow on an active
  Reels tab.

For the close case, the chronological boundary logs should include:

```text
[milo] browser command requested: CloseActiveDistractionTab
[milo] browser command sent: CloseActiveDistractionTab
[milo-native-host] local command received: CloseActiveDistractionTab
[milo-native-host] forwarding command to Firefox
[milo-native-host] command frame written to Firefox
[milo-extension] browser command received: close_active_distraction_tab
[milo-extension] close command: querying active tab
[milo-extension] close command classification: YouTubeShorts
[milo-extension] closing active distraction tab
[milo-extension] tab closed
[milo-extension] close command result: closed
[milo] browser command result: closed
```

After rebuilding the helper, reload the temporary extension so Firefox closes
the old native-host process and launches the current executable. `cargo run`
alone builds the default Milo binary and does not refresh
`target/debug/milo-native-host`.

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
ephemeral. The extension performs no scoring, timers, blocking, automatic tab
closing, content inspection, scroll monitoring, dialogue, or state logic. Its
only browser action is the explicitly requested, immediately revalidated close
of the current active distracting tab.
