use std::{fmt, thread};
use std::error::Error;
use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alsa::Mixer;
use alsa::mixer::{Selem, SelemChannelId, SelemId};
use clap::Parser;
use xcb::Connection;
use xcb::x::{Event, GrabKey, GrabMode, Keycode, ModMask};

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

    /// modifiers for hotkey, use + for multiple e.g. control+mod3
    #[clap(short='m', long, default_value = "mod3", parse(try_from_str = parse_modifiers))]
    hotkey_modifiers: ModMask,

    /// modifiers for hotkey (62 = Left Shift)
    #[clap(short='k', long, default_value_t = 62)] // Shift_Left
    hotkey_keycode: Keycode,
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

    listen_to_keyboard_events_and_update_mixer(expected_capture_state, &args.device, &args.control, args.unmute_delay, args.hotkey_modifiers, args.hotkey_keycode)
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

fn listen_to_keyboard_events_and_update_mixer(expected_capture_state: Arc<AtomicBool>, device: &str, control: &str, unmute_delay_ms: u64, hotkey_modifiers: ModMask, hotkey_keycode: Keycode) {
    let alsa_mixer = Mixer::new(device, false).expect("Failed to setup alsa");
    let mixer_capture_elem = get_alsa_mixer_capture_elem(&alsa_mixer, control).expect("Failed to find recording channel");

    let x_conn = open_x_and_listen_to_hotkey(hotkey_modifiers, hotkey_keycode).expect("Failed to setup X11");

    loop {
        let event = x_conn.wait_for_event();
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
            Event::KeyPress(_) => {
                println!("Unmuting");
                thread::sleep(Duration::from_millis(unmute_delay_ms));
                set_expected_capture_state(&expected_capture_state, &mixer_capture_elem, true);
            }
            Event::KeyRelease(_) => {
                println!("Muting");
                set_expected_capture_state(&expected_capture_state, &mixer_capture_elem, false);
            }
            _ => (),
        }
        // just in case they otherwise pile up somewhere
        alsa_mixer.handle_events().expect("alsa_mixer.handle_events() failed");
    }
}

fn open_x_and_listen_to_hotkey(hotkey_modifiers: ModMask, hotkey_keycode: Keycode) -> Result<Connection, Box<dyn Error>> {
    let (conn, screen_num) = xcb::Connection::connect(None)?;
    let screen = conn.get_setup().roots().nth(screen_num as usize).ok_or(GenericError("Could not find screen"))?;
    let win = screen.root();

    let grab_cookie = conn.send_request_checked(&GrabKey {
        owner_events: true,
        grab_window: win,
        modifiers: hotkey_modifiers,
        key: hotkey_keycode,
        pointer_mode: GrabMode::Async,
        keyboard_mode: GrabMode::Async,
    });
    conn.check_request(grab_cookie).map_err(|e| GenericError(format!("Failed to grab hotkey: {:?}", e)))?;
    Ok(conn)
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
