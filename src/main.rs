use xcb::x::Screen;
use xcb::x::GrabMode;
use xcb::x::ModMask;

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
    conn.flush();

    loop {
        let event = conn.wait_for_event();
        let event = match event {
            Err(e) => { println!("{:#?}", e); break; },
            Ok(e) => e
        };
        println!("{:#?}", event);
    }
}

