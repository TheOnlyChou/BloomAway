# Milo

Milo is a minimal native Wayland desktop companion for GTK4 and Hyprland. It
displays illustrated Idle, Sleeping, Curious, and Concerned animations in a
transparent, undecorated, small normal Wayland toplevel window.

## Native dependency

GTK4 is required. On Arch Linux:

```bash
sudo pacman -S --needed gtk4
```

Milo does not use `gtk4-layer-shell` and does not create a fullscreen or layer
surface.

## Hyprland 0.56 rule

This machine runs Hyprland 0.56.2 and loads Lua configuration from
`~/.config/hypr/hyprland.lua`, with user rules in
`~/.config/hypr/custom/rules.lua`. Add this single Milo-specific rule to the
custom rules file:

```lua
hl.window_rule({
    name = "milo-desktop-companion",
    match = { class = "^com\\.milo\\.desktop$" },
    float = true,
    pin = true,
    no_initial_focus = true,
    decorate = false,
    move = { "(monitor_w-window_w-24)", "(monitor_h-window_h-24)" }
})
```

Reload the config after saving it:

```bash
hyprctl reload
```

The rule matches the `class` field shown by `hyprctl clients`. For this native
Wayland GTK application, that class is its stable GApplication/Wayland app ID,
`com.milo.desktop`. Its stable window title is `Milo`. The class is the better
match because it is intended as application identity rather than display text.

`float` keeps the small toplevel out of the tiling layout, `pin` displays it on
all workspaces, `no_initial_focus` stops it taking startup focus, and
`decorate = false` disables Hyprland's compositor decorations around the
already-undecorated GTK window. `move` is evaluated when the rule is initially
applied: the monitor dimensions minus Milo's actual window dimensions and 24
pixels place it at the bottom-right. Nothing continuously enforces the
position, so a compositor drag leaves Milo where it is dropped.

## Run and inspect

From this directory, in a running Hyprland session:

```bash
cargo run
```

Inspect the mapped window and its class with:

```bash
hyprctl clients
```

Drag Milo with the left mouse button. For development, right-click Milo to
cycle through Idle, Sleeping, Curious, Concerned, and back to Idle. The selected
state is printed in the launching terminal. Press Ctrl+C there to quit.

While Milo is running, focus applications in different activity categories to
see the context transition and visual reaction:

```text
[milo] context changed: Terminal -> Browser
[milo] reaction: Curious
```

## Animation states

All four transparent PNG frames for each state under `assets/milo/` are loaded
once as GDK textures. Milo starts in Idle. Each state owns its frame order and
per-frame durations, and changing state resets that animation to its first
frame and replaces its GLib one-shot timeout. A single `GtkPicture` displays
every state at 128 × 128 logical pixels with aspect-preserving smooth scaling,
so neither the picture nor the window changes size during a switch.

Concerned uses the four normalized frames in `assets/milo/concerned/`, looping
with per-frame durations of 500, 400, 650, and 400 milliseconds through the
same animation player as the other states.

## Automatic idle behavior

Milo opens a small, dedicated Wayland connection and requests an
`ext-idle-notify-v1` notification for the current seat. The development idle
timeout is 60 seconds. When Hyprland sends `Idled`, Milo switches to Sleeping;
when it sends `Resumed`, Milo switches to Curious and a GLib timeout returns it
to Idle after 3 seconds.

The Wayland connection blocks efficiently on its own listener thread. That
thread sends only `Idled` and `Resumed` values through a channel. A task on the
GLib main context receives them and performs every animation and GTK update on
the GTK thread. No mouse coordinates, input devices, `hyprctl` calls, or GTK
objects are polled from the listener.

Right-click remains a manual development override. It cancels a pending
Curious-to-Idle transition and cycles the current state. The next genuine
Wayland idle or resume event returns control to the automatic behavior. If the
compositor does not expose `ext-idle-notify-v1`, Milo prints a diagnostic and
continues running with its Idle animation and right-click switching.

## Active application observation

Milo connects directly to Hyprland's newline-delimited event socket at
`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock`. A dedicated
thread blocks on the socket and forwards only parsed `activewindow` events to a
task on the GLib main context. The event payload is split at its first comma,
so later commas remain part of the title.

Application identity comes from the window class, not its title.
Firefox, Google Chrome, and Chromium are classified as Browser; Code and
Code OSS as Development; kitty and Alacritty as Terminal; Steam as Gaming;
Spotify as Media; and unmatched classes as Other. Matching is
case-insensitive.

Events for `com.milo.desktop` and empty classes are ignored, and the last
non-Milo window remains the meaningful activity context. Identical consecutive
window events are also ignored. A change between two different activity
categories makes Milo Curious for 1.5 seconds before returning to Idle. Changes
within one category, including title-only changes, update the retained window
without triggering a reaction or log.

System idle has priority over application reactions. An app switch cannot wake
Sleeping Milo or replace the existing three-second resume reaction. All
temporary reactions share one cancellable GLib timeout, guarded by a generation
number so an obsolete callback cannot restore Idle after a newer event.

If the event socket is unavailable at startup, Milo reports that activity
tracking is unavailable and continues normally. If an established connection
drops, the listener retries the same socket every two seconds without polling
active-window state. Milo queries `hyprctl activewindow` once after initially
connecting so the first subsequent socket event can be compared with the
application that was already focused at startup.

## Firefox browser activity bridge

Milo also listens on the user-local Unix socket
`$XDG_RUNTIME_DIR/milo/browser.sock`. The socket's parent directory is mode
`0700` and the socket is mode `0600`. A dedicated listener thread accepts
newline-delimited JSON from `milo-native-host`, validates it, and sends only the
parsed `BrowserActivity` through a channel to a task on the GLib main context.
The main-context callback logs each received category, for example:

```text
[milo] browser activity: YouTubeShorts
```

`BrowserActivity` remains deliberately separate from Hyprland's
`ActivityCategory::Browser`: one describes Firefox's selected category and the
other describes the focused desktop application.

Firefox launches only the small `milo-native-host` binary, never the GTK Milo
binary. Native Messaging stdin uses a four-byte native-endian unsigned length
followed by exactly that many UTF-8 JSON bytes. The helper rejects frames above
64 KiB before allocating their payload, writes diagnostics only to stderr, and
keeps reading framed messages until Firefox closes stdin. It retains one Unix
socket connection to Milo; a local connection or write failure is logged and
retried on the next browser message without ending the Firefox connection.
Valid category-only messages cross the local socket as one JSON object per
line. It opens no TCP ports and persists nothing.

Firefox connection loss is represented explicitly on the local protocol as
`{"type":"browser_tracking_unavailable"}`. Milo does not interpret the end of
an individual Unix stream as browser unavailability. This keeps a temporary
local forwarding failure separate from the lifetime of Firefox's persistent
Native Messaging port while preserving the rule that a genuine Firefox/native
host disconnect ends a distraction session.

See [extension/README.md](extension/README.md) for the user-level Firefox host
installation, removal, extension identity/authorization, and exact manual test
procedure. Milo remains usable without Firefox, and Firefox remains usable
without Milo.

## Continuous distraction sessions

One in-memory controller combines the current desktop category, latest browser
category, and system-idle status. A session exists only while the desktop
category is `Browser`, the system is active, and the browser category is either
`YouTubeShorts` or `InstagramReels`. Normal YouTube, Instagram, and all other
pages do not start sessions.

Sessions use `std::time::Instant`, so elapsed time is monotonic and unaffected
by wall-clock adjustments. A one-second GLib timeout checks elapsed time with
negligible wakeups. Each session tracks its next unreported threshold, making
the development thresholds at 10, 20, and 30 seconds fire exactly once even if
one timer check crosses more than one deadline.

Leaving the Browser desktop category, changing to a non-distracting browser
category, losing the Firefox/native-host connection, or becoming system-idle
ends the current session immediately. Changing directly between Shorts and
Reels logs the old session's end and starts a new session at zero. Returning
from another application or system idle likewise starts a fresh continuous
session if the combined context still qualifies. No separated visits are
accumulated or persisted.

Distraction sessions retain their existing logs and also drive persistent
visual severity:

```text
[milo] distraction started: YouTubeShorts
[milo] distraction 10s: YouTubeShorts
[milo] distraction state: Idle -> Curious
[milo] distraction 20s: YouTubeShorts
[milo] distraction state: Curious -> Concerned
[milo] distraction 30s: YouTubeShorts
```

Session start maps to Idle, 10 seconds maps to persistent Curious, and 20
seconds maps to persistent Concerned. At 30 seconds Milo remains Concerned, the
threshold is logged, and a one-time `StillScrolling` intervention is requested.
Ending a session returns Curious or Concerned to Idle. A direct Shorts/Reels
kind change ends the old session and starts the new one at Idle.

`BehaviorController` is the sole authority that applies states to the animator.
Its pure state model resolves priority as system idle/Sleeping first, the
temporary resume Curious reaction second, persistent distraction severity
third, and normal Idle last. App-switch reactions cannot override an active
distraction session. Temporary callbacks carry a generation token and always
recompute the authoritative state, so an obsolete callback cannot replace
Concerned with Idle. System idle clears the session; resume therefore ends at
Idle rather than restoring the old Concerned state.

No threshold closes tabs, blocks sites, changes pages, sends Firefox commands,
or produces scores, story progression, or persistence.

### Still-scrolling intervention

The 30-second intervention is modeled independently from its GTK presentation.
The lifecycle records whether a distraction session is active, whether that
session has already requested its intervention, and whether the intervention
is visible. It emits only `Show(StillScrolling)` and `Hide` presentation
requests. The text and response labels are centralized in `intervention.rs`.

GTK presents the request as a small `GtkPopover` parented to Milo's existing
picture, above the character. It contains `Still scrolling?` plus `Take a break`
and `Keep scrolling` buttons. Button callbacks emit the local semantic
responses `TakeBreak` and `KeepScrolling`, dismiss the popover, and log the
response. Neither response changes the distraction session or communicates
with Firefox, so Milo remains Concerned until the user actually leaves the
distracting context.

The requested-this-session flag remains set after dismissal, preventing the
same continuous session from reopening the intervention. Session end, a direct
Shorts/Reels switch, browser disconnect, and system idle all dismiss a visible
popover and reset eligibility. Only a new session reaching 30 seconds can show
it again.

For temporary presentation diagnostics, start Milo when no other Milo instance
is running with:

```bash
cargo run -- --debug-intervention
```

This bypasses browser and distraction events and calls the GTK presentation
layer after about one second. The normal 30-second path logs, in order,
`intervention requested: StillScrolling`, `intervention presentation: Show`,
and `GTK: calling intervention popover.popup()`. Hide presentation is also
logged. The debug option is development-only and does not fake Firefox input or
alter intervention eligibility.

### Manual distraction-session test

1. Start Milo, load the Firefox extension, and focus YouTube Shorts. Confirm a
   new session starts while Milo remains Idle.
2. At about 10 seconds, confirm Milo changes to persistent Curious.
3. At about 20 seconds, confirm Milo changes to persistent Concerned.
4. At about 30 seconds, confirm Milo remains Concerned and a popover displays
   `Still scrolling?`, `Take a break`, and `Keep scrolling`.
5. Choose `Keep scrolling`; confirm the response log, that the popup closes,
   and that it does not reopen during the same session.
6. Switch to kitty and confirm the session ends and Milo immediately returns to
   Idle.
7. Return to Shorts, reach 30 seconds again, and confirm the new session can
   show the intervention.
8. While it is visible, switch away and confirm it closes immediately.
9. Show it in another session, become system-idle, and confirm the popup closes
   while Milo becomes Sleeping.
10. Resume and confirm Sleeping -> temporary Curious -> Idle; the previous
    Concerned state and intervention must not return.
11. Right-click repeatedly and confirm the debug cycle still includes
    Concerned without answering the intervention.

## How dragging works

`GtkApplicationWindow` creates a normal `GdkSurface` that implements the
`GdkToplevel` interface. On a real left-button press, Milo reads the event's
pointer device, surface-relative coordinates, button number, and timestamp,
then calls `GdkToplevel::begin_move` once. GDK sends the Wayland interactive
move request and Hyprland owns all subsequent pointer tracking and window
movement until release. Milo contains no pointer-motion loop, coordinate
calculation, layer-shell margins, or `hyprctl` movement commands.

The transparent surface still receives pointer input; transparent-pixel
click-through is not implemented. Milo has no persistence, browser URL
tracking, productivity scoring, blocking, or automated movement behavior.
