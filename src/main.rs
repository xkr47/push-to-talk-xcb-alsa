use std::{fmt, thread};
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alsa::Mixer;
use alsa::mixer::{Selem, SelemChannelId, SelemId};
use clap::Parser;
use xcb::Connection;
use xcb::x::{Event, GetModifierMapping, GrabKey, GrabMode, KeyButMask, Keycode, ModMask, Window};

/// Push to talk using X11 hotkey — uses ALSA but works indirectly also with PulseAudio and PipeWire
#[derive(Parser, Clone)]
#[clap(author, version, about)]
struct Args {
    /// alsa device name
    #[clap(short, long, default_value_t = String::from("default"))]
    device: String,

    /// alsa mixer control name
    #[clap(short, long, default_value_t = String::from("Capture"))]
    control: String,

    /// delay unmute by this much time (milliseconds)
    #[clap(short, long, default_value_t = 150)]
    unmute_delay: u64,

    /// modifiers for push hotkey, use + for multiple e.g. control+mod3
    #[clap(short='m', long, default_value = "mod3", parse(try_from_str = parse_modifiers))]
    push_modifiers: ModMask,

    /// modifiers for hotkey (62 = Left Shift)
    #[clap(short='k', long, default_value_t = 62)]
    push_keycode: Keycode,

    /// modifiers for toggle hotkey, use + for multiple e.g. control+mod3
    #[clap(short='M', long, default_value = "mod3+control", parse(try_from_str = parse_modifiers))]
    toggle_modifiers: ModMask,

    /// modifiers for toggle hotkey (62 = Left Shift, 0 to disable)
    #[clap(short='K', long, default_value_t = 62)]
    toggle_keycode: Keycode,
}

fn main() {
    let args: Args = Args::parse();

    let expected_capture_state = Arc::new(AtomicBool::new(false));

    {
        let expected_capture_state = expected_capture_state.clone();
        let args = args.clone();
        thread::spawn(move || {
            enforce_mixer_capture_state(expected_capture_state, &args.device, &args.control)
        });
    }

    listen_to_keyboard_events_and_update_mixer(expected_capture_state, &args.device, &args.control, args.unmute_delay, args.push_modifiers, args.push_keycode, args.toggle_modifiers, args.toggle_keycode)
}

fn parse_modifiers(str: &str) -> Result<ModMask, &'static str> {
    if !str.is_empty() {
        str.split('+')
            .map(parse_modifier)
            .fold(Ok(ModMask::empty()), |acc, x|
                match acc {
                    Ok(prev) => x.map(|cur| prev | cur),
                    Err(e) => Err(e),
                })
    } else {
        Ok(ModMask::empty())
    }
}

fn parse_modifier(str: &str) -> Result<ModMask, &'static str> {
    match str {
        "shift" => Ok(ModMask::SHIFT),
        "lock" => Ok(ModMask::LOCK),
        "control" => Ok(ModMask::CONTROL),
        "mod1" => Ok(ModMask::N1),
        "mod2" => Ok(ModMask::N2),
        "mod3" => Ok(ModMask::N3),
        "mod4" => Ok(ModMask::N4),
        "mod5" => Ok(ModMask::N5),
        _ => Err("expected modifier: `shift`, `lock`, `control`, `mod1`, `mod2`, `mod3`, `mod4`, or `mod5`"),
    }
}

// -------------

fn enforce_mixer_capture_state(expected_capture_state: Arc<AtomicBool>, device: &str, control: &str) -> ! {
    let alsa_mixer = Mixer::new(device, false).expect("Failed to setup alsa");
    let mixer_capture_elem = get_alsa_mixer_capture_elem(&alsa_mixer, control).expect("Failed to find recording channel");
    loop {
        let actual = get_unanimous_capture_state(&mixer_capture_elem).expect("Could not get capture switch value");
        let expected = expected_capture_state.load(Ordering::Acquire);
        if actual != Some(expected) {
            println!("Fixing capture state to {}", if expected { "unmuted" } else { "muted" });
            if let Err(e) = set_capture_state(&mixer_capture_elem, expected) {
                println!("- Error fixing: {:?}", e);
            }
        }
        alsa_mixer.wait(None).expect("alsa_mixer.wait() failed");
        alsa_mixer.handle_events().expect("alsa_mixer.handle_events() failed");
    }
}

fn get_alsa_mixer_capture_elem<'a>(alsa_mixer: &'a Mixer, control: &str) -> Result<Selem<'a>, Box<dyn Error>> {
    let mixer_capture_elem = alsa_mixer.find_selem(&SelemId::new(control, 0)).ok_or_else(|| GenericError(format!("Could not find simple control {}", control)))?;
    if !mixer_capture_elem.has_capture_switch() {
        return Err(GenericError("Capture switch not found, cannot adjust").into());
    }
    Ok(mixer_capture_elem)
}

fn get_unanimous_capture_state(mixer_capture_elem: &Selem<'_>) -> Result<Option<bool>, Box<dyn Error>> {
    let mut channels = SelemChannelId::all().iter();
    let first_channel_state = 0 != mixer_capture_elem.get_capture_switch(*channels.next().unwrap())?;
    for channel in channels {
        let state = 0 != mixer_capture_elem.get_capture_switch(*channel)?;
        if state != first_channel_state {
            return Ok(None)
        }
    }
    Ok(Some(first_channel_state))
}

fn set_capture_state(mixer_capture_elem: &Selem<'_>, state: bool) -> Result<(), Box<dyn Error>> {
    for channel in SelemChannelId::all() {
        mixer_capture_elem.set_capture_switch(*channel, state.into())?;
    }
    Ok(())
}

// -------------

#[allow(clippy::too_many_arguments)]
fn listen_to_keyboard_events_and_update_mixer(expected_capture_state: Arc<AtomicBool>, device: &str, control: &str, unmute_delay_ms: u64, push_modifiers: ModMask, push_keycode: Keycode, toggle_modifiers: ModMask, toggle_keycode: Keycode) {
    let alsa_mixer = Mixer::new(device, false).expect("Failed to setup alsa");
    let mixer_capture_elem = get_alsa_mixer_capture_elem(&alsa_mixer, control).expect("Failed to find recording channel");

    let x_conn = open_x_and_listen_to_hotkeys(push_modifiers, push_keycode, toggle_modifiers, toggle_keycode).expect("Failed to setup X11");

    // in case keycode is a modifier we need to have adjusted modifiers for release events
    let keycode_to_modifier = get_modifier_mapping(&x_conn);
    let push_release_modifiers = push_modifiers | keycode_to_modifier.get(&push_keycode).cloned().unwrap_or_else(ModMask::empty);
    let toggle_release_modifiers = toggle_modifiers | keycode_to_modifier.get(&toggle_keycode).cloned().unwrap_or_else(ModMask::empty);
    drop(keycode_to_modifier);

    let push_press_match = (KeyButMask::from_bits_truncate(push_modifiers.bits()), push_keycode);
    let toggle_press_match = (KeyButMask::from_bits_truncate(toggle_modifiers.bits()), toggle_keycode);
    let push_release_match = (KeyButMask::from_bits_truncate(push_release_modifiers.bits()), push_keycode);
    let toggle_release_match = (KeyButMask::from_bits_truncate(toggle_release_modifiers.bits()), toggle_keycode);

    // don't immediately unmute on release after muting on press
    let mut mute_pending_release = false;

    let mut next_event_maybe: Option<xcb::Result<xcb::Event>> = None;
    loop {
        let event = next_event_maybe.take().unwrap_or_else(|| x_conn.wait_for_event());
        thread::sleep(Duration::from_millis(15)); // sometimes the next press event of a repeated event arrives some 3..6ms later
        next_event_maybe = match x_conn.poll_for_event() {
            Ok(Some(x)) => Some(Ok(x)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        };
        #[allow(unreachable_patterns)]
            let event = match event {
            Err(e) => {
                println!("Error, exiting — {:#?}", e);
                break;
            }
            Ok(xcb::Event::X(e)) => e,
            Ok(e) => {
                println!("Unsupported event, exiting — {:#?}", e);
                break;
            }
        };
        match event {
            Event::KeyPress(evt) => {
                let m = (evt.state(), evt.detail());
                #[allow(clippy::collapsible_if)]
                if m == push_press_match {
                    println!("Unmuting by push-press");
                    unmute(&expected_capture_state, unmute_delay_ms, &mixer_capture_elem);
                } else if m == toggle_press_match {
                    if expected_capture_state.load(Ordering::Acquire) {
                        println!("Muting by toggle-press");
                        mute(&expected_capture_state, &mixer_capture_elem);
                        mute_pending_release = true;
                    }
                }
            }
            Event::KeyRelease(evt) => {
                // skip repeated key events (e.g. Pause key)
                if let Some(Ok(xcb::Event::X(Event::KeyPress(press_evt)))) = &next_event_maybe {
                    if press_evt.detail() == evt.detail() && press_evt.time() == evt.time() {
                        next_event_maybe = None; // skip both next press event and ..
                        continue; // current release event
                    }
                }

                let m = (evt.state(), evt.detail());
                if m == push_release_match {
                    println!("Muting by push-release");
                    mute(&expected_capture_state, &mixer_capture_elem);
                } else if m == toggle_release_match {
                    if mute_pending_release {
                        mute_pending_release = false;
                    } else if !(expected_capture_state.load(Ordering::Acquire)) {
                        println!("Unmuting by toggle-release");
                        unmute(&expected_capture_state, unmute_delay_ms, &mixer_capture_elem);
                    }
                }
            }
            _ => (),
        }
        // just in case they otherwise pile up somewhere
        alsa_mixer.handle_events().expect("alsa_mixer.handle_events() failed");
    }
}

fn get_modifier_mapping(x_conn: &Connection) -> HashMap<Keycode, ModMask> {
    let cookie = x_conn.send_request(&GetModifierMapping {});
    let reply = x_conn.wait_for_reply(cookie).map_err(|e| GenericError(format!("Failed to get keyboard mapping: {:?}", e))).unwrap();
    let keycodes_per_modifier = reply.keycodes().len() / 8;
    reply.keycodes()
        .chunks_exact(keycodes_per_modifier)
        .zip(&[
            ModMask::SHIFT,
            ModMask::LOCK,
            ModMask::CONTROL,
            ModMask::N1,
            ModMask::N2,
            ModMask::N3,
            ModMask::N4,
            ModMask::N5,
        ])
        .flat_map(|(keycodes, mask)| keycodes.iter()
            .filter(|keycode| **keycode != 0)
            .map(|keycode| (*keycode, *mask)))
        .collect()
}

fn mute(expected_capture_state: &Arc<AtomicBool>, mixer_capture_elem: &Selem) {
    set_expected_capture_state(expected_capture_state, mixer_capture_elem, false);
}

fn unmute(expected_capture_state: &Arc<AtomicBool>, unmute_delay_ms: u64, mixer_capture_elem: &Selem) {
    thread::sleep(Duration::from_millis(unmute_delay_ms));
    set_expected_capture_state(expected_capture_state, mixer_capture_elem, true);
}

fn open_x_and_listen_to_hotkeys(push_modifiers: ModMask, push_keycode: Keycode, toggle_modifiers: ModMask, toggle_keycode: Keycode) -> Result<Connection, Box<dyn Error>> {
    let (conn, screen_num) = xcb::Connection::connect(None)?;
    let screen = conn.get_setup().roots().nth(screen_num as usize).ok_or(GenericError("Could not find screen"))?;
    let win = screen.root();

    listen_to_hotkey(push_modifiers, push_keycode, &conn, win)?;
    if toggle_keycode != 0 {
        listen_to_hotkey(toggle_modifiers, toggle_keycode, &conn, win)?;
    }
    Ok(conn)
}

fn listen_to_hotkey(modifiers: ModMask, keycode: Keycode, x_conn: &Connection, win: Window) -> Result<(), Box<dyn Error>> {
    let grab_cookie = x_conn.send_request_checked(&GrabKey {
        owner_events: true,
        grab_window: win,
        modifiers,
        key: keycode,
        pointer_mode: GrabMode::Async,
        keyboard_mode: GrabMode::Async,
    });
    x_conn.check_request(grab_cookie).map_err(|e| GenericError(format!("Failed to grab hotkey: {:?}", e)))?;
    Ok(())
}

fn set_expected_capture_state(expected_capture_state: &Arc<AtomicBool>, mixer_capture_elem: &Selem, state: bool) {
    expected_capture_state.store(state, Ordering::Release);
    if let Err(e) = set_capture_state(mixer_capture_elem, state) {
        print!("Error setting mixer capture state: {:?}", e);
    }
}

// -------------

#[derive(Debug)]
struct GenericError<S: AsRef<str> + Display + Debug>(S);

impl<S: AsRef<str> + Display + Debug> Error for GenericError<S> {}

impl<S: AsRef<str> + Display + Debug> Display for GenericError<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GenericError: {}", self.0)
    }
}
