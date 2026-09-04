# Milo

Milo is currently a minimal native Wayland overlay prototype for GTK4 and
Hyprland. It displays a transparent, undecorated window with a 140 × 80 minimum
size around a temporary `Milo 🐈` label.

## Native dependency

On Arch Linux, install the GTK layer-shell library before building:

```bash
sudo pacman -S --needed gtk4-layer-shell
```

GTK4 itself is also required (`gtk4`), but it was already installed when this
prototype was created.

## Run

From this directory, in a running Hyprland session:

```bash
cargo run
```

Milo should appear at x=50, y=50 from the monitor's top-left corner. Drag the
label with the left mouse button to move it, and press Ctrl+C in the launching
terminal to quit.

## How it works

`gtk4-layer-shell` turns the `GtkApplicationWindow` into a Wayland layer
surface before it is shown. Milo anchors only to the top and left edges. Those
two margins represent its x/y position and are updated by a GTK drag gesture.
Startup does not depend on monitor detection or GTK allocation timing. During a
drag, once the surface is mapped and has a positive allocation, its position is
clamped using the current monitor geometry and Milo's allocated size.

Milo uses the overlay layer, reserves no screen space, and requests no keyboard
input. Minimal CSS removes the themed window background, border, and shadow;
only the label is drawn. The small synchronous `ctrlc` crate lets Ctrl+C request
an orderly GTK application shutdown; it does not add an async runtime.

The transparent surface still receives pointer input; transparent-pixel
click-through is not implemented. The dragged position is not saved between
runs. Milo has no sprites, animation, tracking, configuration, or other
companion behavior yet.
