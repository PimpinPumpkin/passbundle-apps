//! The tray icon (the menu bar on macOS) and its menu. The agent has no
//! windows of its own: "Open PassBundle" starts the app installed next to it
//! (a separate process, which ends when its window closes), and anything else
//! visual opens in the web vault or the extension.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::{brand, config::Config, reminders::Reminders, version::Version};

#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("../../brand/agent/tray-macos.png");
#[cfg(not(target_os = "macos"))]
const ICON: &[u8] = include_bytes!("../../brand/agent/tray-windows.png");

enum UserEvent {
    Menu(MenuEvent),
    /// What the tray says about items expiring changed (reminders.rs).
    Reminders(Option<String>),
}

/// The tooltip: the name, and the reminder when there is one.
fn tooltip(reminder: Option<&str>) -> String {
    match reminder {
        Some(words) => format!("{}: {words}", brand::NAME),
        None => brand::NAME.to_owned(),
    }
}

pub fn run(config: Config, reminders: Arc<Reminders>) -> ! {
    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        // A menu bar app: no Dock icon and no app menu.
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();
    let told = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));
    reminders.listen(move |words| {
        let _ = told.send_event(UserEvent::Reminders(words));
    });

    let title = MenuItem::new(
        format!("{} {}", brand::NAME, Version::current()),
        false,
        None,
    );
    // The app, when it's installed next to the agent.
    let app = std::env::current_exe()
        .ok()
        .map(|agent| app_next_to(&agent))
        .filter(|app| app.is_file());
    let open_app = MenuItem::new(format!("Open {}", brand::NAME), true, None);
    let open = MenuItem::new("Open web vault", true, None);
    let quit = MenuItem::new(format!("Quit {}", brand::NAME), true, None);
    // "2 items expire soon", first in the menu while there's something to
    // say; it opens the app (or the web vault, where there's no app).
    let reminder = MenuItem::new("", true, None);
    let reminder_id = reminder.id().clone();
    let mut reminding = false;
    let mut menu_shown: Option<Menu> = None;
    let mut tray: Option<TrayIcon> = None;
    let mut remind = move |menu: &Menu, tray: &TrayIcon, words: Option<String>| {
        match &words {
            Some(text) => {
                reminder.set_text(text);
                if !reminding && menu.prepend(&reminder).is_ok() {
                    reminding = true;
                }
            }
            None => {
                if reminding && menu.remove(&reminder).is_ok() {
                    reminding = false;
                }
            }
        }
        let _ = tray.set_tooltip(Some(tooltip(words.as_deref())));
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // macOS needs the event loop running before a status item exists.
            Event::NewEvents(StartCause::Init) => {
                let menu = Menu::new();
                menu.append_items(&[&title, &PredefinedMenuItem::separator()])
                    .expect("tray menu");
                if app.is_some() {
                    menu.append(&open_app).expect("tray menu");
                }
                menu.append_items(&[&open, &quit]).expect("tray menu");
                let builder = TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .with_tooltip(brand::NAME);
                // macOS draws template icons in the menu bar's own colors.
                #[cfg(target_os = "macos")]
                let builder = builder.with_icon_templated(icon());
                #[cfg(not(target_os = "macos"))]
                let builder = builder.with_icon(icon());
                let built = builder.build().expect("tray icon");
                // Anything the app said before the tray was there.
                if let Some(words) = reminders.shown() {
                    remind(&menu, &built, Some(words));
                }
                tray = Some(built);
                menu_shown = Some(menu);
            }
            Event::UserEvent(UserEvent::Reminders(words)) => {
                if let (Some(menu), Some(tray)) = (&menu_shown, &tray) {
                    remind(menu, tray, words);
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if event.id == reminder_id {
                    match &app {
                        Some(app) => launch_app(app),
                        None => open_url(&config.server_url),
                    }
                } else if event.id == open_app.id() {
                    if let Some(app) = &app {
                        launch_app(app);
                    }
                } else if event.id == open.id() {
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

/// The app installed with the agent: `passbundle` next to it (Windows), or on
/// macOS the app whose helper the agent is
/// (`<App>.app/Contents/Helpers/<Name> Agent.app/Contents/MacOS/passbundle-agent`).
fn app_next_to(agent: &Path) -> PathBuf {
    let sibling = agent.with_file_name(format!("passbundle{}", std::env::consts::EXE_SUFFIX));
    #[cfg(target_os = "macos")]
    if !sibling.exists() {
        // Up from MacOS, Contents, the helper app, and Helpers, to the app's Contents.
        if let Some(contents) = agent.ancestors().nth(5) {
            return contents.join("MacOS/passbundle");
        }
    }
    sibling
}

/// Opens the app in its own process: closing its window ends it, and the agent stays.
fn launch_app(app: &Path) {
    // On macOS as the Mac opens apps, so it comes to the front with its own Dock icon.
    #[cfg(target_os = "macos")]
    if let Some(bundle) = app
        .ancestors()
        .nth(3)
        .filter(|bundle| bundle.extension() == Some("app".as_ref()))
    {
        let _ = Command::new("open").arg(bundle).spawn();
        return;
    }
    let _ = Command::new(app).spawn();
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
    fn the_app_is_the_passbundle_program_next_to_the_agent() {
        let agent =
            Path::new("install").join(format!("passbundle-agent{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(
            app_next_to(&agent),
            Path::new("install").join(format!("passbundle{}", std::env::consts::EXE_SUFFIX))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_a_mac_the_app_is_the_one_whose_helper_the_agent_is() {
        let agent = Path::new("/Applications/PassBundle.app/Contents/Helpers/PassBundle Agent.app/Contents/MacOS/passbundle-agent");
        assert_eq!(
            app_next_to(agent),
            Path::new("/Applications/PassBundle.app/Contents/MacOS/passbundle")
        );
    }

    #[test]
    fn the_tooltip_says_the_reminder_after_the_name() {
        assert_eq!(tooltip(None), brand::NAME);
        assert_eq!(
            tooltip(Some("2 items expire soon")),
            format!("{}: 2 items expire soon", brand::NAME)
        );
    }

    #[test]
    fn the_tray_icon_decodes() {
        let (rgba, width, height) = decode_png(ICON).unwrap();
        assert_eq!(rgba.len() as u32, width * height * 4);
        assert!(width >= 32 && width == height);
    }
}
