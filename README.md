![logo](push-to-talk-xcb-alsa.jpeg)

# push-to-talk-xcb-alsa
Push-To-Talk using global hotkey in X11, muting/unmuting an [alsa](https://alsa-project.org/) mixer recording control. Unmutes the microphone when you press the configured hotkey (by default Hyper_L + Shift_R) and mutes it back when you release it. It also monitors the muting state and fixes it in case it changes unexpectedly, for example when plugging a headset into your laptop and the headset mute status happens to be wrong. It has been tested to be working well together with [PipeWire](https://pipewire.org/) / [PulseAudio](https://www.freedesktop.org/wiki/Software/PulseAudio/) at least in Firefox 34, since PipeWire is configured to expose a "PipeWire" alsa device that just does the right thing.

# Installing
1. [Install Rust](https://www.rust-lang.org/)
2. Clone this repo

# Configuring
Adjust the hardcoded settings in the constants in [src/main.rs](src/main.rs) to your liking:

```rust
const DEVICE: &str = "default";
const CONTROL: &str = "Capture";
const UNMUTE_DELAY_MS: u64 = 150;
const HOTKEY_MODIFIERS: ModMask = ModMask::N3; // Hyper_L in my setup
const HOTKEY_KEYCODE: Keycode = 62; // Shift_R
```

1. The `CONTROL` values can be found out using `amixer controls -D default` where `default` is the alsa device name specified in DEVICE.
2. The `UNMUTE_DELAY_MS` is to silence a possible sound created by clicking the hotkey.. but not too much to not mute yourself when you start talking. Unfortunately it is not possible to do the same when releasing the key to mute yourself, so you'll have to be careful not to release it too loudly :)
3. For `HOTKEY_MODIFIERS`, see [ModMask enum values here](https://rust-x-bindings.github.io/rust-xcb/branches/v1.0-dev/xcb/x/struct.ModMask.html) and your modifier mappings using the `xmodmap` command:
```
$ xmodmap
xmodmap:  up to 4 keys per modifier, (keycodes in parentheses):

shift       Shift_L (0x32),  Shift_R (0x3e)
lock      
control     Control_L (0x25),  Control_R (0x69),  Control_L (0x85),  Control_R (0x87)
mod1        Alt_L (0x40),  Alt_L (0xcc)
mod2        Mode_switch (0x6c),  Mode_switch (0x86),  Mode_switch (0xcb)
mod3        Hyper_L (0x42),  Hyper_L (0xcf)
mod4      
mod5      
```
➔ since I want to use `Hyper_L` as modifier, I thus need to use `mod3` which represented by `ModMask::N3`. You can combine multiple modifiers with the `|` operator like `ModMask::CONTROL | ModMask::SHIFT`. To just use a single dedicated hotkey without modifiers, use `ModMask::empty()`.

4. For `HOTKEY_KEYCODE`, see keycodes from e.g. `xev` output
```
$ xev -event keyboard
KeyPress event, serial 28, synthetic NO, window 0x6400001,
    root 0x7cf, subw 0x0, time 58544270, (115,90), root:(1050,594),
    state 0x0, keycode 62 (keysym 0xffe2, Shift_R), same_screen YES,
    XLookupString gives 0 bytes: 
    XmbLookupString gives 0 bytes: 
    XFilterEvent returns: False
```
➔ the "keycode 62" part is the interesting one for the HOTKEY_KEYCODE

# Running
In the cloned repo, run:
```
$ cargo run --release
```

# Credits

* https://stackoverflow.com/qu7estions/4037230/global-hotkey-with-x11-xlib
* https://crates.io/crates/xcb
* https://crates.io/crates/alsa
