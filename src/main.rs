use xcb::x::Screen;
use xcb::x::GrabMode;
use xcb::x::ModMask;
use xcb::x::Event;

/*
amixer -R sget Capture | grep Left: | egrep -o '\bon\b|\boff\b'
amixer -R sset Capture cap
amixer -R sset Capture nocap
 */

fn main() {
    let (conn, screen_num) = xcb::Connection::connect(None).unwrap();
    let screen: &Screen = conn.get_setup().roots().nth(screen_num as usize).unwrap();
    let win = screen.root();

    conn.send_request(&xcb::x::GrabKey {
        owner_events: true,
        grab_window: win,
        modifiers: ModMask::N3,
        key: 0x3e, // Shift_R
        pointer_mode: GrabMode::Async,
        keyboard_mode: GrabMode::Async,
    });
    conn.flush().expect("flush failed");

    loop {
        let event = conn.wait_for_event();
        #[allow(unreachable_patterns)]
        let event = match event {
            Err(e) => {
                println!("Error, exiting — {:#?}", e);
                break;
            },
            Ok(xcb::Event::X(e)) => e,
            Ok(e) => {
                println!("Unsupported event, exiting — {:#?}", e);
                break;
            },
        };
        match event {
            Event::KeyPress(event) => {
                println!("Press {:#?}", event);
            },
            Event::KeyRelease(event) => {
                println!("Release {:#?}", event);
            },
            _ => {
                println!("Unhandled event, ignoring — {:#?}", event);
            }
        }
    }
}

