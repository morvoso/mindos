# mindwm

The MindOS compositor: a Smithay-based Wayland/XWayland compositor with
game-mode window placement and the built-in Mind bar (launcher, shell and
chat with the MindOS mind daemon).

    cargo build --release
    ./target/release/mindwm --winit       # nested window for development
    ./target/release/mindwm --tty-udev    # DRM/KMS session (from a TTY)

Keybindings, the Mind bar and the `/etc/mindos/mindwm.toml` schema are
documented in `../docs/COMPOSITOR.md`.
