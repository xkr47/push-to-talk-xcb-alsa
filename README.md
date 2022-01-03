# push-to-talk-xcb-alsa
Push-To-Talk using global hotkey in X11, muting/unmuting alsa mixer control

# Installing
1. [Install Rust](https://www.rust-lang.org/)
2. Clone this repo

# Configuring
Adjust the hardcoded settings in the constants in [src/main.rs](blob/main/src/main.rs) to your liking:

```rust
const DEVICE: &str = "default";
const CONTROL: &str = "Capture";
const UNMUTE_DELAY_MS: u64 = 150;
const HOTKEY_MODIFIERS: ModMask = ModMask::N3; // Hyper_L in my setup
const HOTKEY_KEYCODE: Keycode = 0x3e; // Shift_R
```

# Running
4. Run `cargo run --release` in the cloned repo
