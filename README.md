# Milo

Milo is a minimal native Wayland desktop-companion experiment for GTK4 and
Hyprland. It displays only a transparent, undecorated `Milo 🐈` label in a
small normal Wayland toplevel window.

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

Drag Milo with the left mouse button. Press Ctrl+C in the launching terminal
to quit.

## How dragging works

`GtkApplicationWindow` creates a normal `GdkSurface` that implements the
`GdkToplevel` interface. On a real left-button press, Milo reads the event's
pointer device, surface-relative coordinates, button number, and timestamp,
then calls `GdkToplevel::begin_move` once. GDK sends the Wayland interactive
move request and Hyprland owns all subsequent pointer tracking and window
movement until release. Milo contains no pointer-motion loop, coordinate
calculation, layer-shell margins, or `hyprctl` movement commands.

The transparent surface still receives pointer input; transparent-pixel
click-through is not implemented. Milo has no sprites, animation, persistence,
tracking, or other companion behavior yet.
