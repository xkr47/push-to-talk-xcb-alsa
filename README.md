![logo](push-to-talk-xcb-alsa.jpeg)

# push-to-talk-xcb-alsa
Push-To-Talk using global hotkey in X11, muting/unmuting an [alsa](https://alsa-project.org/) mixer recording control. Unmutes the microphone when you press the configured hotkey (by default Hyper_L + Shift_R) and mutes it back when you release it. It also monitors the muting state and fixes it in case it changes unexpectedly, for example when plugging a headset into your laptop and the headset mute status happens to be wrong.

It has been tested to be working well together with [PipeWire](https://pipewire.org/) / [PulseAudio](https://www.freedesktop.org/wiki/Software/PulseAudio/) at least in Fedora 34, since PipeWire is configured to expose a "PipeWire" alsa device that just does the right thing. Also seems to work on Devuan 3 with PulseAudio.

# Installing
1. [Install Rust](https://www.rust-lang.org/)
2. Install alsa & xcb development packages
  * `alsa-lib-devel` and `libxcb-devel` for Fedora-based distributions
  * `libasound2-dev` and `libxcb-dev` for Debian-based distributions
3. Clone this repo
4. In the cloned repo, run `cargo compile --release`

# Configuring
Use commandline arguments to adjust which device, mixer control, unmute delay, hotkey etc settings you want to use.
Run `cargo run --release -- --help` to get a list of available options.

1. The `--control <control>` values can be found out using `amixer scontrols -D default` where `default` is the alsa device name specified in DEVICE:
```
$ amixer scontrols -D default 
Simple mixer control 'Master',0
Simple mixer control 'Capture',0
```
2. The `--unmute-delay <delay>` is to silence a possible sound created by clicking the hotkey.. but not too much to not mute yourself when you start talking. Unfortunately it is not possible to do the same when releasing the key to mute yourself, so you'll have to be careful not to release it too loudly :)
3. For `--hotkey-modifiers <modifiers>`, see your modifier mappings using the `xmodmap` command:
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
The modifiers are in the first column.

➔ since I want to use `Hyper_L` as modifier, I thus need to use `mod3`. You can combine multiple modifiers by adding `+` between them like `control+shift`. To just use a single dedicated hotkey without modifiers, use `--hotkey_modifiers ""`.

4. Easiest is to use `--hotkey-keysym <keysym>` with e.g. `Shift_R` as `<keysym>`. This will enable all keycodes that map to `<keysym>`. For single keycodes use `--hotkey-keycode <keycode>` instead, see keycodes from e.g. `xev` output and and pressing the key you want to use while pointing at the window:
```
$ xev -event keyboard
KeyPress event, serial 28, synthetic NO, window 0x6400001,
    root 0x7cf, subw 0x0, time 58544270, (115,90), root:(1050,594),
    state 0x0, keycode 62 (keysym 0xffe2, Shift_R), same_screen YES,
    XLookupString gives 0 bytes: 
    XmbLookupString gives 0 bytes: 
    XFilterEvent returns: False
```
➔ the "keycode 62" part is the interesting one so you should use `--hotkey-keycode 62` in this case. 

## Example

```
$ cargo run --release -- --push-modifiers shift --push-keysym KP_Enter --toggle-modifiers control+shift --toggle-keysym KP_Enter
```

# Running
In the cloned repo, run:
```
$ cargo run --release --
```

You can add options to the end of the command if needed. Use `--help` for help.

# Credits

* https://stackoverflow.com/questions/4037230/global-hotkey-with-x11-xlib
* https://crates.io/crates/xcb
* https://crates.io/crates/alsa
* https://crates.io/crates/xkb
