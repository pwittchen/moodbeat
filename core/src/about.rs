//! The About item of the app menu.
//!
//! macOS shows the standard About panel with styled credits (clickable links, centered
//! lines) that Tauri's `AboutMetadata` can't express; other platforms use Tauri's dialog.

use tauri::menu::{Menu, MenuEvent};
use tauri::{AppHandle, Wry};

const AUTHOR: &str = "Piotr Wittchen";
const AUTHOR_URL: &str = "https://wittchen.io";
const REPO_URL: &str = "https://github.com/pwittchen/moodbeat";

#[cfg(target_os = "macos")]
const ABOUT_ID: &str = "about";

/// Tauri's default menu with the About item swapped for ours.
pub fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    #[cfg(target_os = "macos")]
    {
        use tauri::menu::MenuItem;
        let about = MenuItem::with_id(
            app,
            ABOUT_ID,
            format!("About {}", app.package_info().name),
            true,
            None::<&str>,
        )?;
        // The About item comes first in the app submenu.
        if let Some(submenu) = menu.items()?.into_iter().find_map(|item| item.as_submenu().cloned()) {
            submenu.remove_at(0)?;
            submenu.insert(&about, 0)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri::menu::{AboutMetadata, PredefinedMenuItem, HELP_SUBMENU_ID};
        let pkg = app.package_info();
        let metadata = AboutMetadata {
            name: Some(pkg.name.clone()),
            version: Some(pkg.version.to_string()),
            authors: Some(vec![format!("{AUTHOR} ({AUTHOR_URL})")]),
            comments: Some(pkg.description.into()),
            website: Some(REPO_URL.into()),
            website_label: Some("GitHub".into()),
            icon: app.default_window_icon().cloned(),
            ..Default::default()
        };
        let about = PredefinedMenuItem::about(app, None, Some(metadata))?;
        if let Some(submenu) = menu.get(HELP_SUBMENU_ID).and_then(|item| item.as_submenu().cloned()) {
            submenu.remove_at(0)?;
            submenu.insert(&about, 0)?;
        }
    }
    Ok(menu)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "signature required by Builder::on_menu_event"
)]
pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    #[cfg(target_os = "macos")]
    if event.id() == ABOUT_ID {
        let name = app.package_info().name.clone();
        let version = app.package_info().version.to_string();
        let shown = app.run_on_main_thread(move || macos::show_about_panel(&name, &version));
        if let Err(e) = shown {
            tracing::error!("failed to show the About panel: {e}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, event);
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{
        NSAboutPanelOptionApplicationIcon, NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion,
        NSAboutPanelOptionCredits, NSAboutPanelOptionVersion, NSApplication, NSColor, NSFont, NSFontAttributeName,
        NSForegroundColorAttributeName, NSImage, NSLinkAttributeName, NSMutableParagraphStyle,
        NSParagraphStyleAttributeName, NSTextAlignment,
    };
    use objc2_foundation::{NSAttributedString, NSData, NSDictionary, NSMutableAttributedString, NSString, NSURL};

    use super::{AUTHOR, AUTHOR_URL, REPO_URL};

    const FONT_SIZE: f64 = 12.0;
    const LINE_SPACING: f64 = 6.0;

    /// Must run on the main thread.
    pub fn show_about_panel(name: &str, version: &str) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let credits = NSMutableAttributedString::new();
        for (i, line) in [
            Line::Text(format!("version {version}")),
            Line::Link(REPO_URL.trim_start_matches("https://").into(), REPO_URL),
            Line::Text(format!("made by {AUTHOR}")),
            Line::Link(AUTHOR_URL.trim_start_matches("https://").into(), AUTHOR_URL),
        ]
        .into_iter()
        .enumerate()
        {
            let text = if i == 0 {
                line.text().to_owned()
            } else {
                format!("\n{}", line.text())
            };
            credits.appendAttributedString(&attributed(&text, line.url()));
        }

        // The version lives in the credits (lowercase); blank values hide the built-in line.
        let empty = NSString::new();
        let mut keys: Vec<&NSString> = unsafe {
            vec![
                NSAboutPanelOptionApplicationName,
                NSAboutPanelOptionApplicationVersion,
                NSAboutPanelOptionVersion,
                NSAboutPanelOptionCredits,
            ]
        };
        let mut objects: Vec<Retained<AnyObject>> = vec![
            Retained::into_super(Retained::into_super(NSString::from_str(name))),
            Retained::into_super(Retained::into_super(empty.clone())),
            Retained::into_super(Retained::into_super(empty)),
            Retained::into_super(Retained::into_super(Retained::into_super(credits))),
        ];
        // Unbundled dev builds would otherwise show a generic folder icon.
        let icon_data = NSData::with_bytes(include_bytes!("../icons/128x128@2x.png"));
        if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &icon_data) {
            keys.push(unsafe { NSAboutPanelOptionApplicationIcon });
            objects.push(Retained::into_super(Retained::into_super(icon)));
        }
        let options = NSDictionary::from_retained_objects(&keys, &objects);
        let app = NSApplication::sharedApplication(mtm);
        unsafe { app.orderFrontStandardAboutPanelWithOptions(&options) };
    }

    enum Line {
        Text(String),
        Link(String, &'static str),
    }

    impl Line {
        fn text(&self) -> &str {
            match self {
                Line::Text(t) | Line::Link(t, _) => t,
            }
        }

        fn url(&self) -> Option<&'static str> {
            match self {
                Line::Text(_) => None,
                Line::Link(_, url) => Some(url),
            }
        }
    }

    fn attributed(text: &str, url: Option<&str>) -> Retained<NSAttributedString> {
        let style = NSMutableParagraphStyle::new();
        style.setAlignment(NSTextAlignment::Center);
        style.setParagraphSpacing(LINE_SPACING);

        let mut keys: Vec<&NSString> = unsafe {
            vec![
                NSFontAttributeName,
                NSParagraphStyleAttributeName,
                NSForegroundColorAttributeName,
            ]
        };
        let mut values: Vec<Retained<AnyObject>> = vec![
            Retained::into_super(Retained::into_super(NSFont::systemFontOfSize(FONT_SIZE))),
            Retained::into_super(Retained::into_super(Retained::into_super(style))),
            Retained::into_super(Retained::into_super(if url.is_some() {
                NSColor::linkColor()
            } else {
                NSColor::secondaryLabelColor()
            })),
        ];
        if let Some(link) = url.and_then(|u| NSURL::URLWithString(&NSString::from_str(u))) {
            keys.push(unsafe { NSLinkAttributeName });
            values.push(Retained::into_super(Retained::into_super(link)));
        }
        let attrs = NSDictionary::from_retained_objects(&keys, &values);
        unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(text), &attrs) }
    }
}
