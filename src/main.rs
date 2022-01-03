use std::{fmt, thread};
use std::error::Error;
use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alsa::Mixer;
use alsa::mixer::{Selem, SelemChannelId, SelemId};
use xcb::Connection;
use xcb::x::Event;
use xcb::x::GrabMode;
use xcb::x::ModMask;
use xcb::x::Screen;

const CONTROL: &str = "Capture";
const UNMUTE_DELAY_MS: u64 = 150;

fn main() {
    let x_conn = open_x_and_listen_to_hotkey().expect("Failed to setup X11");

    let expected_capture_state = Arc::new(AtomicBool::new(false));
    {
        let expected_capture_state = expected_capture_state.clone();
        thread::spawn(move || {
            let alsa_mixer = Mixer::new("default", false).expect("Failed to setup alsa");
            let selem = get_alsa_mixer_capture_elem(&alsa_mixer).expect("Failed to find recording channel");
            loop {
                let capture_enabled = 0 != selem.get_capture_switch(SelemChannelId::FrontLeft).expect("Could not get capture switch value");
                println!("capture_enabled = {}", capture_enabled);
                if capture_enabled != expected_capture_state.load(Ordering::Acquire) {
                    let expected = !capture_enabled;
                    println!("fixing to {}", expected);
                    if let Err(e) = set_capture_state(&selem, expected) {
                        print!("Error fixing: {:?}", e);
                    }
                }
                alsa_mixer.wait(None).expect("alsa_mixer.wait() failed");
                alsa_mixer.handle_events().expect("alsa_mixer.handle_events() failed");
            }
        });
    }

    let alsa_mixer = Mixer::new("default", false).expect("Failed to setup alsa");
    let selem = get_alsa_mixer_capture_elem(&alsa_mixer).expect("Failed to find recording channel");
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
                expected_capture_state.store(true, Ordering::Release);
                if let Err(e) = set_capture_state(&selem, true) {
                    print!("Error setting mixer capture state: {:?}", e);
                }
            }
            Event::KeyRelease(_) => {
                println!("Muting");
                expected_capture_state.store(false, Ordering::Release);
                if let Err(e) = set_capture_state(&selem, false) {
                    print!("Error setting mixer capture state: {:?}", e);
                }
            }
            _ => {
                println!("Unhandled event, ignoring — {:#?}", event);
            }
        }
    }
}

fn set_capture_state(selem: &Selem<'_>, state: bool) -> Result<(), Box<dyn Error>> {
    let state = if state { 1 } else { 0 };
    for channel in SelemChannelId::all() {
        selem.set_capture_switch(*channel, state)?;
    }
    Ok(())
}

fn get_alsa_mixer_capture_elem<'a>(alsa_mixer: &'a Mixer) -> Result<Selem<'a>, Box<dyn Error>> {
    let selem = alsa_mixer.find_selem(&SelemId::new(CONTROL, 0)).ok_or_else(|| GenericError(format!("Could not find simple control {}", CONTROL)))?;
    if !selem.has_capture_switch() {
        Err(GenericError("Capture switch not found, cannot adjust"))?;
    }
    Ok(selem)
}

#[derive(Debug)]
struct GenericError<S: AsRef<str> + Display + Debug>(S);

impl<S: AsRef<str> + Display + Debug> Error for GenericError<S> {}

impl<S: AsRef<str> + Display + Debug> Display for GenericError<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GenericError: {}", self.0)
    }
}

fn open_x_and_listen_to_hotkey() -> Result<Connection, Box<dyn Error>> {
    let (conn, screen_num) = xcb::Connection::connect(None)?;
    let screen: &Screen = conn.get_setup().roots().nth(screen_num as usize).ok_or(GenericError("Could not find screen"))?;
    let win = screen.root();

    conn.send_request(&xcb::x::GrabKey {
        owner_events: true,
        grab_window: win,
        modifiers: ModMask::N3,
        key: 0x3e, // Shift_R
        pointer_mode: GrabMode::Async,
        keyboard_mode: GrabMode::Async,
    });
    conn.flush()?;
    Ok(conn)
}
