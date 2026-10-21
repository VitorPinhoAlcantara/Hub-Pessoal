use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{PlatformError, TrayEvent, TrayHandler, TrayImage};

pub fn build(
    image: TrayImage,
    tooltip: &str,
    handler: TrayHandler,
) -> Result<TrayIcon, PlatformError> {
    let error = |reason: String| PlatformError::Launch {
        target: "bandeja".to_string(),
        reason,
    };
    let icon =
        Icon::from_rgba(image.rgba, image.width, image.height).map_err(|e| error(e.to_string()))?;
    let open = MenuItem::new("Abrir o hub", true, None);
    let quit = MenuItem::new("Sair", true, None);
    let menu = Menu::new();
    menu.append(&open).map_err(|e| error(e.to_string()))?;
    menu.append(&quit).map_err(|e| error(e.to_string()))?;
    let open_id = open.id().clone();
    let quit_id = quit.id().clone();
    let handler = std::sync::Arc::new(handler);
    let menu_handler = handler.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == open_id {
            menu_handler(TrayEvent::Open);
        } else if event.id == quit_id {
            menu_handler(TrayEvent::Quit);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            handler(TrayEvent::Open);
        }
    }));
    TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip(tooltip)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| error(e.to_string()))
}
