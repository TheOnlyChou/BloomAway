# Milo Firefox extension

This development-only WebExtension classifies the active Firefox tab as one
of `YouTube`, `YouTubeShorts`, `Instagram`, `InstagramReels`, or `Other`. It
prints category transitions locally in the extension background console. It
does not communicate with Milo's Rust application.

## Permissions

The Manifest V3 extension requests only `tabs`. Firefox requires that
permission for a background script to read `Tab.url`. There are no host
permissions, content scripts, injected code, network requests, or persistent
storage.

## Load temporarily in Firefox

1. Open `about:debugging` in Firefox.
2. Select **This Firefox**.
3. Select **Load Temporary Add-on**.
4. Choose `extension/manifest.json` from this repository.
5. Find **Milo Browser Activity** in the temporary extensions list and select
   **Inspect** to open its background console.

After changing either extension file, use **Reload** on the extension's
`about:debugging` entry. A temporary extension is removed when Firefox exits.

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

## URL classification

`classifyUrl(url)` uses the built-in `URL` parser and exact host matching.
Only HTTP and HTTPS pages on `youtube.com`, `www.youtube.com`, `instagram.com`,
or `www.instagram.com` receive site categories. YouTube paths beginning with
`/shorts/` become `YouTubeShorts`; Instagram paths beginning with `/reel/` or
`/reels/` become `InstagramReels`. Missing, malformed, internal, and all other
URLs become `Other`.

## Manual test cases

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

No browsing history is collected, URLs are not persisted or transmitted, and
the extension performs no scoring, blocking, intervention, content inspection,
scroll monitoring, or Native Messaging.
