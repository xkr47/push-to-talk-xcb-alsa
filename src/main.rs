use std::{fmt, thread};
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use alsa::Mixer;
use alsa::mixer::{Selem, SelemChannelId, SelemId};
use chrono::Local;
use clap::Parser;
use xcb::Connection;
use xcb::x::{Event, GetKeyboardMapping, GetModifierMapping, GrabKey, GrabMode, KeyButMask, Keycode, Keysym, ModMask, Window};

/// Push to talk using X11 hotkey — uses ALSA but works indirectly also with PulseAudio and PipeWire
#[derive(Parser, Clone, Debug)]
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

    /// keycode for push hotkey (62 = Left Shift)
    #[clap(short='c', long, default_value_t = 62, group="push-key")]
    push_keycode: Keycode,

    /// modifiers for push hotkey (62 = Left Shift)
    #[clap(short='s', long, group="push-key")]
    push_keysym: Option<String>,

    /// modifiers for toggle hotkey, use + for multiple e.g. control+mod3
    #[clap(short='M', long, default_value = "mod3+control", parse(try_from_str = parse_modifiers))]
    toggle_modifiers: ModMask,

    /// keycode for toggle hotkey (62 = Left Shift, 0 to disable)
    #[clap(short='C', long, default_value_t = 62, group="toggle-key")]
    toggle_keycode: Keycode,

    /// keysym for toggle hotkey ("Shift_L")
    #[clap(short='S', long, group="toggle-key")]
    toggle_keysym: Option<String>,
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

    listen_to_keyboard_events_and_update_mixer(expected_capture_state, &args.device, &args.control, args.unmute_delay,
                                               args.push_modifiers, args.push_keycode, args.push_keysym,
                                               args.toggle_modifiers, args.toggle_keycode, args.toggle_keysym)
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
    let mut alsa_mixer = Mixer::new(device, false).expect("Failed to setup alsa");
    loop {
        let result = get_alsa_mixer_capture_elem(&alsa_mixer, control);
        if result.is_err() {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        let mixer_capture_elem = result.expect("Failed to find recording channel");
        let actual = get_unanimous_capture_state(&mixer_capture_elem).expect("Could not get capture switch value");
        let expected = expected_capture_state.load(Ordering::Acquire);
        if actual != Some(expected) {
            println!("{} Fixing capture state to {}", log_timestamp(), if expected { "unmuted" } else { "muted" });
            if let Err(e) = set_capture_state(&mixer_capture_elem, expected) {
                println!("{} - Error fixing: {:?}", log_timestamp(), e);
            }
        }
        let before = Instant::now();
        alsa_mixer.wait(None).expect("alsa_mixer.wait() failed");
        alsa_mixer.handle_events().expect("alsa_mixer.handle_events() failed");
        if Instant::now().duration_since(before) > Duration::from_millis(1000) {
            alsa_mixer = Mixer::new(device, false).expect("Failed to setup alsa");
        }
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
fn listen_to_keyboard_events_and_update_mixer(expected_capture_state: Arc<AtomicBool>, device: &str, control: &str, unmute_delay_ms: u64, push_modifiers: ModMask, push_keycode: Keycode, push_keysym: Option<String>, toggle_modifiers: ModMask, toggle_keycode: Keycode, toggle_keysym: Option<String>) {
    let alsa_mixer = Mixer::new(device, false).expect("Failed to setup alsa");
    let mixer_capture_elem = get_alsa_mixer_capture_elem(&alsa_mixer, control).expect("Failed to find recording channel");
    let (x_conn, win) = open_x().expect("Failed to setup X11");

    let mut keyboard_mapping = None;

    let push_keycodes = get_keycodes_for_keysym(&x_conn, &mut keyboard_mapping, push_keycode, push_keysym).unwrap();
    let toggle_keycodes = get_keycodes_for_keysym(&x_conn, &mut keyboard_mapping, toggle_keycode, toggle_keysym).unwrap();

    drop(keyboard_mapping);

    listen_to_hotkey(push_modifiers, &push_keycodes, &x_conn, win).unwrap();
    listen_to_hotkey(toggle_modifiers, &toggle_keycodes, &x_conn, win).unwrap();

    // in case keycode is a modifier we need to have adjusted modifiers for release events
    let keycode_to_modifier = get_modifier_mapping(&x_conn).unwrap();

    let push_release_modifiers = push_keycodes.iter().map(|keycode| push_modifiers | keycode_to_modifier.get(keycode).cloned().unwrap_or_else(ModMask::empty));
    let toggle_release_modifiers = toggle_keycodes.iter().map(|keycode| toggle_modifiers | keycode_to_modifier.get(keycode).cloned().unwrap_or_else(ModMask::empty));

    #[derive(Debug, Clone)]
    enum KeyAction {
        Push,
        Toggle,
    }

    let push_press_entries = push_keycodes.iter().map(|keycode| ((from_mod_mask(push_modifiers), *keycode), KeyAction::Push));
    let toggle_press_entries = toggle_keycodes.iter().map(|keycode| ((from_mod_mask(toggle_modifiers), *keycode), KeyAction::Toggle));
    let push_release_entries = push_keycodes.iter().zip(push_release_modifiers).map(|(keycode, modifiers)| ((from_mod_mask(modifiers), *keycode), KeyAction::Push));
    let toggle_release_entries = toggle_keycodes.iter().zip(toggle_release_modifiers).map(|(keycode, modifiers)| ((from_mod_mask(modifiers), *keycode), KeyAction::Toggle));

    let press_map = try_collect_map(push_press_entries.chain(toggle_press_entries)).unwrap();
    let release_map = try_collect_map(push_release_entries.chain(toggle_release_entries)).unwrap();

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
                println!("{} Error, exiting — {:#?}", log_timestamp(), e);
                break;
            }
            Ok(xcb::Event::X(e)) => e,
            Ok(e) => {
                println!("{} Unsupported event, exiting — {:#?}", log_timestamp(), e);
                break;
            }
        };
        match event {
            Event::KeyPress(evt) => {
                match press_map.get(&(evt.state(), evt.detail())) {
                    Some(KeyAction::Push) => {
                        println!("{} Unmuting by push-press", log_timestamp());
                        unmute(&expected_capture_state, unmute_delay_ms, &mixer_capture_elem);
                    },
                    Some(KeyAction::Toggle) => {
                        if expected_capture_state.load(Ordering::Acquire) {
                            println!("{} Muting by toggle-press", log_timestamp());
                            mute(&expected_capture_state, &mixer_capture_elem);
                            mute_pending_release = true;
                        }
                    },
                    _ => ()
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

                match release_map.get(&(evt.state(), evt.detail())) {
                    Some(KeyAction::Push) => {
                        println!("{} Muting by push-release", log_timestamp());
                        mute(&expected_capture_state, &mixer_capture_elem);
                    },
                    Some(KeyAction::Toggle) => {
                        if mute_pending_release {
                            mute_pending_release = false;
                        } else if !(expected_capture_state.load(Ordering::Acquire)) {
                            println!("{} Unmuting by toggle-release", log_timestamp());
                            unmute(&expected_capture_state, unmute_delay_ms, &mixer_capture_elem);
                        }
                    },
                    _ => ()
                }
            }
            _ => (),
        }
        // just in case they otherwise pile up somewhere
        alsa_mixer.handle_events().expect("alsa_mixer.handle_events() failed");
    }
}

fn try_collect_map<K: Debug + Eq + Hash, V: Debug, I: Iterator<Item = (K, V)>>(mut entries: I) -> Result<HashMap<K, V>, GenericError<&'static str>> {
    entries
        .try_fold(HashMap::new(), |mut map, (k, v)|
            if map.insert(k, v).is_none() {
                Ok(map)
            } else {
                Err(GenericError("Conflicting keybindings"))
            })
}

fn from_mod_mask(modifiers: ModMask) -> KeyButMask {
    KeyButMask::from_bits_truncate(modifiers.bits())
}

fn get_keyboard_mapping_reverse(x_conn: &Connection) -> Result<HashMap<Keysym, Vec<Keycode>>, Box<dyn Error>> {
    let first_keycode = 8;
    let cookie = x_conn.send_request(&GetKeyboardMapping {
        first_keycode,
        count: 248,
    });
    let reply = x_conn.wait_for_reply(cookie).map_err(|e| GenericError(format!("Failed to get keyboard mapping: {:?}", e)))?;
    let mut map: HashMap<Keysym, Vec<Keycode>> = HashMap::new();
    reply.keysyms()
        .chunks_exact(reply.keysyms_per_keycode().into())
        .zip(first_keycode..)
        .for_each(|(keysyms, keycode)| {
            let mut keysyms = keysyms.to_vec();
            keysyms.sort_unstable();
            keysyms.dedup();
            keysyms.iter()
                .filter(|keysym| **keysym != 0)
                .for_each(|keysym| map.entry(*keysym).or_default().push(keycode))
        });
    Ok(map)
}

fn get_modifier_mapping(x_conn: &Connection) -> Result<HashMap<Keycode, ModMask>, Box<dyn Error>> {
    let cookie = x_conn.send_request(&GetModifierMapping {});
    let reply = x_conn.wait_for_reply(cookie).map_err(|e| GenericError(format!("Failed to get modifier mapping: {:?}", e)))?;
    let keycodes_per_modifier = reply.keycodes().len() / 8;
    Ok(reply.keycodes()
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
        .collect())
}

fn mute(expected_capture_state: &Arc<AtomicBool>, mixer_capture_elem: &Selem) {
    set_expected_capture_state(expected_capture_state, mixer_capture_elem, false);
}

fn unmute(expected_capture_state: &Arc<AtomicBool>, unmute_delay_ms: u64, mixer_capture_elem: &Selem) {
    thread::sleep(Duration::from_millis(unmute_delay_ms));
    set_expected_capture_state(expected_capture_state, mixer_capture_elem, true);
}

fn open_x() -> Result<(Connection, Window), Box<dyn Error>> {
    let (x_conn, screen_num) = xcb::Connection::connect(None)?;
    let screen = x_conn.get_setup().roots().nth(screen_num as usize).ok_or(GenericError("Could not find screen"))?;
    let root = screen.root();
    Ok((x_conn, root))
}

fn get_keycodes_for_keysym(x_conn: &Connection, keyboard_mapping: &mut Option<HashMap<Keysym, Vec<Keycode>>>, keycode: Keycode, keysym: Option<String>) -> Result<Vec<Keycode>, Box<dyn Error>> {
    if let Some(keysym) = keysym {
        let keysym = xkb::Keysym::from_str(&keysym).map_err(|_| GenericError(format!("Unknown keysym '{}'", keysym)))?;
        if keyboard_mapping.is_none() {
            let map = get_keyboard_mapping_reverse(x_conn)?;
            *keyboard_mapping = Some(map);
        }
        let keyboard_mapping: &HashMap<Keysym, Vec<Keycode>> = keyboard_mapping.as_ref().unwrap();
        Ok(keyboard_mapping.get(&keysym.into()).ok_or_else(|| GenericError(format!("No keycode bound to keysym '{}'", keysym)))?.to_vec())
    } else {
        Ok(vec![keycode])
    }
}

fn listen_to_hotkey(modifiers: ModMask, keycodes: &[Keycode], x_conn: &Connection, win: Window) -> Result<(), Box<dyn Error>> {
    #[allow(clippy::needless_collect)]
    let grab_cookies = keycodes.iter().map(|keycode| x_conn.send_request_checked(&GrabKey {
        owner_events: true,
        grab_window: win,
        modifiers,
        key: *keycode,
        pointer_mode: GrabMode::Async,
        keyboard_mode: GrabMode::Async,
    })).collect::<Vec<_>>();

    grab_cookies.into_iter()
        .try_for_each(|cookie|
            x_conn.check_request(cookie).map_err(|e| GenericError(format!("Failed to grab hotkey: {:?}", e)).into()))
}

fn set_expected_capture_state(expected_capture_state: &Arc<AtomicBool>, mixer_capture_elem: &Selem, state: bool) {
    expected_capture_state.store(state, Ordering::Release);
    if let Err(e) = set_capture_state(mixer_capture_elem, state) {
        println!("{} Error setting mixer capture state: {:?}", log_timestamp(), e);
        panic!("Failure to set mixer caputre state");
    }
}

// -------------

fn log_timestamp() -> String {
    format!("{}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
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
