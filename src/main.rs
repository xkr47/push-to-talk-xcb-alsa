use std::{fmt, thread};
use std::error::Error;
use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alsa::Mixer;
use alsa::mixer::{Selem, SelemChannelId, SelemId};
use xcb::Connection;
use xcb::x::{Event, GrabKey, GrabMode, Keycode, ModMask, Screen};

const DEVICE: &str = "default";
const CONTROL: &str = "Capture";
const UNMUTE_DELAY_MS: u64 = 150;
const HOTKEY_MODIFIERS: ModMask = ModMask::N3; // Hyper_L in my setup
const HOTKEY_KEYCODE: Keycode = 0x3e; // Shift_R

fn main() {
    let expected_capture_state = Arc::new(AtomicBool::new(false));

    {
        let expected_capture_state = expected_capture_state.clone();
        thread::spawn(move || {
            enforce_mixer_capture_state(expected_capture_state)
        });
    }

    listen_to_keyboard_events_and_update_mixer(expected_capture_state)
}

// -------------

fn enforce_mixer_capture_state(expected_capture_state: Arc<AtomicBool>) -> ! {
    let alsa_mixer = Mixer::new(DEVICE, false).expect("Failed to setup alsa");
    let mixer_capture_elem = get_alsa_mixer_capture_elem(&alsa_mixer).expect("Failed to find recording channel");
    loop {
        let capture_enabled = 0 != mixer_capture_elem.get_capture_switch(SelemChannelId::FrontLeft).expect("Could not get capture switch value");
        let expected = expected_capture_state.load(Ordering::Acquire);
        if capture_enabled != expected {
            println!("Fixing capture state to {}", if expected { "unmuted" } else { "muted" });
            if let Err(e) = set_capture_state(&mixer_capture_elem, expected) {
                println!("- Error fixing: {:?}", e);
            }
        }
        alsa_mixer.wait(None).expect("alsa_mixer.wait() failed");
        alsa_mixer.handle_events().expect("alsa_mixer.handle_events() failed");
    }
}

fn get_alsa_mixer_capture_elem(alsa_mixer: &Mixer) -> Result<Selem, Box<dyn Error>> {
    let mixer_capture_elem = alsa_mixer.find_selem(&SelemId::new(CONTROL, 0)).ok_or_else(|| GenericError(format!("Could not find simple control {}", CONTROL)))?;
    if !mixer_capture_elem.has_capture_switch() {
        return Err(GenericError("Capture switch not found, cannot adjust").into());
    }
    Ok(mixer_capture_elem)
}

fn set_capture_state(mixer_capture_elem: &Selem<'_>, state: bool) -> Result<(), Box<dyn Error>> {
    let state = if state { 1 } else { 0 };
    for channel in SelemChannelId::all() {
        mixer_capture_elem.set_capture_switch(*channel, state)?;
    }
    Ok(())
}

// -------------

fn listen_to_keyboard_events_and_update_mixer(expected_capture_state: Arc<AtomicBool>) {
    let alsa_mixer = Mixer::new(DEVICE, false).expect("Failed to setup alsa");
    let mixer_capture_elem = get_alsa_mixer_capture_elem(&alsa_mixer).expect("Failed to find recording channel");

    let x_conn = open_x_and_listen_to_hotkey().expect("Failed to setup X11");

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
                thread::sleep(Duration::from_millis(UNMUTE_DELAY_MS));
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

fn open_x_and_listen_to_hotkey() -> Result<Connection, Box<dyn Error>> {
    let (conn, screen_num) = xcb::Connection::connect(None)?;
    let screen: &Screen = conn.get_setup().roots().nth(screen_num as usize).ok_or(GenericError("Could not find screen"))?;
    let win = screen.root();

    let grab_cookie = conn.send_request_checked(&GrabKey {
        owner_events: true,
        grab_window: win,
        modifiers: HOTKEY_MODIFIERS,
        key: HOTKEY_KEYCODE,
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
