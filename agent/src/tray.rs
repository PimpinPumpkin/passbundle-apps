//! The tray icon (the menu bar on macOS) and its menu. Version 1 has no
//! windows: anything visual opens in the web vault or the extension.

use std::process::Command;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::{brand, config::Config, version::Version};

#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("../../brand/agent/tray-macos.png");
#[cfg(not(target_os = "macos"))]
const ICON: &[u8] = include_bytes!("../../brand/agent/tray-windows.png");

enum UserEvent {
    Menu(MenuEvent),
}

pub fn run(config: Config) -> ! {
    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        // A menu bar app: no Dock icon and no app menu.
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let title = MenuItem::new(
        format!("{} {}", brand::NAME, Version::current()),
        false,
        None,
    );
    let open = MenuItem::new("Open web vault", true, None);
    let quit = MenuItem::new(format!("Quit {}", brand::NAME), true, None);
    let mut tray: Option<TrayIcon> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // macOS needs the event loop running before a status item exists.
            Event::NewEvents(StartCause::Init) => {
                let menu = Menu::new();
                menu.append_items(&[&title, &PredefinedMenuItem::separator(), &open, &quit])
                    .expect("tray menu");
                let builder = TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_tooltip(brand::NAME);
                // macOS draws template icons in the menu bar's own colors.
                #[cfg(target_os = "macos")]
                let builder = builder.with_icon_templated(icon());
                #[cfg(not(target_os = "macos"))]
                let builder = builder.with_icon(icon());
                tray = Some(builder.build().expect("tray icon"));
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if event.id == open.id() {
                    open_url(&config.server_url);
                } else if event.id == quit.id() {
                    tray.take();
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    })
}

fn icon() -> Icon {
    let (rgba, width, height) = decode_png(ICON).expect("tray icon PNG");
    Icon::from_rgba(rgba, width, height).expect("tray icon")
}

/// Decodes an 8-bit RGBA PNG, as brand/render-assets.sh writes them.
fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), png::DecodingError> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let frame = reader.next_frame(&mut buffer)?;
    buffer.truncate(frame.buffer_size());
    assert_eq!(
        (frame.color_type, frame.bit_depth),
        (png::ColorType::Rgba, png::BitDepth::Eight),
        "tray icons are 8-bit RGBA"
    );
    Ok((buffer, frame.width, frame.height))
}

/// Opens a URL the config already checked is HTTPS, in the default browser.
fn open_url(url: &str) {
    #[cfg(windows)]
    let result = Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = result {
        eprintln!("couldn't open {url}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tray_icon_decodes() {
        let (rgba, width, height) = decode_png(ICON).unwrap();
        assert_eq!(rgba.len() as u32, width * height * 4);
        assert!(width >= 32 && width == height);
    }
}
