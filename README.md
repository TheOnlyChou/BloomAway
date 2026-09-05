# Milo

Milo is a minimal native Wayland desktop companion for GTK4 and Hyprland. It
displays illustrated Idle, Sleeping, Curious, Concerned, PlayWithYarn, Stretch,
Grooming, and LookingAround animations in a transparent, undecorated, small
normal Wayland toplevel window.

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
cycle through Idle, Sleeping, Curious, Concerned, PlayWithYarn, Stretch,
Grooming, LookingAround, and back to Idle. The selected state is printed in the
launching terminal. Press Ctrl+C there to quit.

While Milo is running, focus applications in different activity categories to
see the context transition and visual reaction:

```text
[milo] context changed: Terminal -> Browser
[milo] reaction: Curious
```

## Animation states

All transparent PNG frames for each state under `assets/milo/` are loaded once
as GDK textures. Milo starts in Idle. Each state owns its frame order and
per-frame durations, and changing state resets that animation to its first
frame and replaces its GLib one-shot timeout. A single `GtkPicture` displays
every state at 128 × 128 logical pixels with aspect-preserving smooth scaling,
so neither the picture nor the window changes size during a switch.

Concerned uses the four normalized frames in `assets/milo/concerned/`, looping
with per-frame durations of 500, 400, 650, and 400 milliseconds through the
same animation player as the other states.

Four cozy, non-critical activities use eight unchanged 384 × 512 frames each:

- PlayWithYarn: 180, 180, 180, 220, 180, 220, 220, and 260 milliseconds;
  three loops.
- Stretch: 220, 220, 240, 280, 420, 260, 220, and 320 milliseconds; one loop.
- Grooming: 260, 240, 280, 320, 300, 320, 260, and 340 milliseconds; two loops.
- LookingAround: 360, 260, 260, 360, 520, 280, 300, and 400 milliseconds; one
  loop.

One cozy scheduler arms its first opportunity 120–300 seconds after startup
and chooses a new randomized delay after every completed, interrupted, or
rejected opportunity. At an eligible opportunity it chooses approximately
equally among all four activities while excluding the last selected activity,
so consecutive cozy runs do not repeat. The last choice is in-memory only. The
same lightweight time- and process-seeded generator supplies both choices and
delays without adding a randomness dependency or a fixed repeating interval.

An autonomous opportunity starts only from calm authoritative Idle: the system
is awake, no distraction or Curious reaction is active, neither intervention
nor narrative UI is active, and no debug state is being displayed. Autonomous
Each autonomous activity runs for its configured complete loop count without a
separate completion timer. The animation player then notifies the behavior
controller, which recomputes Sleeping, Curious, Concerned, or Idle from current
conditions. Any real behavior or UI event interrupts the active cozy activity
immediately, and a generation guard prevents stale completion from overwriting
the newer state.

All four cozy animations remain manually inspectable in the right-click cycle.
Debug versions loop until another right-click advances the cycle or a real
behavior event takes control. They do not emit cozy scheduler start/completion
events or narrative triggers.

### Manual cozy-activity test

1. Start Milo and work normally without Shorts/Reels or right-clicking it.
2. Confirm one future cozy opportunity is scheduled and Milo does not start an
   activity immediately.
3. Over several calm opportunities, confirm Milo selects different activities
   from PlayWithYarn, Stretch, Grooming, and LookingAround without immediately
   repeating one.
4. Confirm each activity completes after its configured loops and returns
   naturally to Idle.
5. During a later autonomous run, trigger system idle and confirm Milo switches
   immediately to Sleeping without a delayed return to Idle.
6. Repeat with distraction severity or an intervention and confirm the
   authoritative behavior wins immediately.

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
newline-delimited JSON from `milo-native-host`, validates it, and sends parsed
browser events through a channel to a task on the GLib main context. The same
persistent socket carries narrow browser commands in the opposite direction.
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

Commands from Milo are newline-delimited on the local socket, then a dedicated
native-host writer thread serializes them to Firefox as four-byte
native-endian-length-prefixed JSON. That thread is the sole owner of stdout, so
frames cannot interleave and diagnostics remain on stderr.

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

No threshold automatically closes tabs, blocks sites, changes pages, or
produces scores. The 20-second semantic threshold may produce the one-time
narrative line described below, without changing distraction behavior.

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
response. `KeepScrolling` sends no browser command. `TakeBreak` asks the browser
bridge to send the single semantic command `CloseActiveDistractionTab`; if no
native host is currently connected, the command is discarded rather than
queued. Neither response directly changes the distraction session.

Milo logs `browser command requested` before the bridge call and `browser
command sent` only after the local Unix write succeeds. The helper logs local
receipt, Firefox forwarding, and successful framed stdout completion. Command
results return through Firefox Native Messaging stdin and the Unix socket and
are logged by Milo without affecting `MiloState`.

Firefox remains the final authority. On receiving the command, the extension
freshly queries the focused Firefox window and its current active tab, runs the
current URL through the existing classifier, and calls `tabs.remove` only for
`YouTubeShorts` or `InstagramReels`. Normal YouTube, Instagram, other pages,
missing tabs, and malformed commands are ignored. This final check protects a
normal tab selected after the intervention appeared from a stale close command.
After a successful close, ordinary active-tab tracking reports the newly
selected context and the distraction controller reacts to that real state.

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
    Concerned and all four cozy animations without answering the intervention.

### Manual browser-command tests

1. Reach the intervention on Shorts and choose `Keep scrolling`. Confirm the
   popup closes, the tab remains open, Milo stays Concerned, and no browser
   command is logged.
2. Start a fresh Shorts session, reach the intervention, and choose `Take a
   break`. Confirm Milo logs `CloseActiveDistractionTab`, the active Shorts tab
   closes, and subsequent `BrowserActivity` naturally ends or continues the
   session according to the newly selected tab.
3. Reach the intervention, switch Firefox to a normal page before the action
   is processed, and trigger `TakeBreak` if possible. Confirm the normal tab is
   not closed and Milo logs the `ignored_not_distracting` result.
4. Repeat the successful close test with Instagram Reels.

## Minimal narrative progression

`NarrativeEngine` observes only semantic events: first launch, the first real
20-second distraction threshold, an accepted break, and a return proven by
system idle followed by resume. It does not access GTK, animation state,
Firefox, or either browser transport. In particular, right-clicking to cycle
Milo to Concerned does not emit a narrative trigger.

Narrative progress contains `introduction_seen`, `concerned_dialogue_seen`,
`breaks_accepted`, `return_dialogue_seen`, and `eli_revealed`, plus a small
nested `world` record for the photograph. It is stored as JSON at:

```text
$XDG_STATE_HOME/bloomaway/narrative.json
```

When `XDG_STATE_HOME` is unset, Milo uses
`~/.local/state/bloomaway/narrative.json`. Saves use a same-directory temporary
file followed by rename. Missing, unreadable, or malformed state falls back to
fresh progress with a diagnostic; load and save failures never stop Milo. No
URL, title, application history, or browser history is persisted.

An accepted break sets an in-memory pending flag. A subsequent genuine system
idle marks that break as observed, and only the following resume can emit
`ReturnedAfterBreak`. Closing or changing a browser tab alone is not accepted
as proof of taking a break.

Narrative dialogue uses a separate buttonless `GtkPopover` attached above
Milo. Lines display sequentially for four seconds each and dismiss after the
last line. A tiny in-memory line queue keeps ordering deterministic. The
intervention popover has priority: showing it pauses and hides narrative
dialogue, while hiding it resumes the interrupted line. Consequently a
`BreakAccepted` sequence queued by a button response appears only after the
intervention has closed.

At startup, Milo presents its window before arming the introduction. The
picture's GTK `map` signal (or its already-mapped state when `present()` maps
synchronously) schedules a one-shot GLib idle callback on the main context.
Only that callback emits `FirstLaunch`, so the narrative popover is attached,
mapped, and ready before the introduction is persisted and displayed.

The initial narrative content is deliberately limited to:

- First launch: `Hi.` then `You work here?`
- First persistent Concerned event: `You've been staring at that for a while.`
- First break: `Good.` then `...I mean, I'll be fine.`
- First return after a proven break: `You came back.` then `I wasn't waiting.`
- Second break: `Going somewhere?` then `...Good.`
- Third break: `Found something earlier.`, `It has a name on it.`, then
  `...Eli.`

The third break sets `eli_revealed`; later breaks increment the persisted count
without adding new dialogue yet. Existing Chapter 1 progress files without a
`world` record remain valid. If such a file already has `eli_revealed`, Milo
migrates it to a pending, still-hidden photograph rather than skipping the new
away-cycle reveal.

## The Photograph

`world.rs` is a deliberately small world-object layer. It currently knows only
`WorldObject::EliPhoto`, its persistent progress flags, and semantic events; it
does not know about GTK, animation, distraction sessions, Firefox, or Native
Messaging. The world fields are stored inside the existing narrative JSON:

- `eli_photo_pending`
- `eli_photo_visible`
- `eli_photo_inspected`
- `eli_photo_appearance_dialogue_seen`

The third accepted break marks the photograph pending and saves that state, but
does not show it. Once pending, only an authoritative system-idle event followed
by resume can emit `WorldEvent::EliPhotoAppeared`. Browser tab closure, ordinary
application switching, elapsed wall time, and cozy activities do not count as
an away cycle. The appeared event persists visibility before the application
layer reveals the GTK object and queues `I left it here.` followed by `Thought
you might want to see it.` This appearance happens once; after restart the photo
is restored directly from persisted visibility and the dialogue is not
replayed.

The GTK window uses a compact horizontal container: Milo keeps his existing
128-pixel picture and a 46-by-56-pixel paper-card placeholder sits beside him
only after it is unlocked. The placeholder is a normal button with replaceable
child content, so final transparent artwork can be introduced without changing
world logic. Clicking it for the first time persists inspection and queues
`That's me.`, `The other name is Eli.`, and `...I haven't seen this in a long
time.` Later clicks intentionally do nothing.

World dialogue goes through the existing narrative queue. An intervention
therefore remains highest priority and temporarily hides/pauses dialogue rather
than allowing popovers to overlap. Narrative activity may interrupt any
autonomous cozy activity through the existing presentation priority, but
clicking the photo does not directly choose an animation state or change cozy
scheduling.

### Manual Photograph test

1. Reach the third accepted break. Confirm its existing Eli dialogue and the
   `world object pending: EliPhoto` log, with no photograph visible.
2. Restart Milo if desired; confirm the photograph remains pending and hidden.
3. Let the real system-idle timeout fire, then resume. Confirm the
   `EliPhotoAppeared` and visible logs, the small card beside Milo, and the two
   appearance lines.
4. Restart Milo. Confirm the photograph remains visible and the appearance
   lines do not replay.
5. Click the photograph. Confirm the three inspection lines and the
   `world object inspected: EliPhoto` log.
6. Restart and click again. Confirm the photograph remains visible and the
   inspection sequence does not replay.

### Manual narrative test

1. Move any existing `narrative.json` aside, then start Milo. Confirm Milo is
   visible and the logs show `window presented`, `Milo mapped`, and `narrative
   startup ready` before `FirstLaunch`. Confirm `Hi.` and `You work here?`
   appear in order and do not repeat after restarting Milo.
2. Reach Concerned through a real 20-second Shorts/Reels session. Confirm its
   one-time line appears. Confirm the right-click debug cycle does not trigger
   it.
3. Reach the intervention and choose `Take a break`. Confirm the existing tab
   close still occurs, then confirm the first-break sequence appears after the
   intervention closes.
4. After accepting a break, become system-idle and resume. Confirm the return
   sequence appears once. Confirm ordinary idle/resume without a preceding
   accepted break produces no return dialogue.
5. Accept second and third breaks in fresh distraction sessions and confirm
   their sequences, including the name `Eli`. Confirm the photograph is only
   pending until a real idle/resume cycle. Restart Milo and verify the progress
   does not repeat earlier milestones.

## How dragging works

`GtkApplicationWindow` creates a normal `GdkSurface` that implements the
`GdkToplevel` interface. On a real left-button press, Milo reads the event's
pointer device, surface-relative coordinates, button number, and timestamp,
then calls `GdkToplevel::begin_move` once. GDK sends the Wayland interactive
move request and Hyprland owns all subsequent pointer tracking and window
movement until release. Milo contains no pointer-motion loop, coordinate
calculation, layer-shell margins, or `hyprctl` movement commands.

The transparent surface still receives pointer input; transparent-pixel
click-through is not implemented. Milo persists only the small narrative and
world progress record described above. It has no persisted browser URLs,
productivity scoring, blocking, or automated movement behavior.
