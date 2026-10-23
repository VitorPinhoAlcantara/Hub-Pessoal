use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{PlatformError, TrayEvent, TrayHandler, TrayImage, TrayMenuState};

const MUTE_HOURS: [u32; 5] = [1, 2, 4, 8, 12];

static PENDING: Mutex<Option<TrayMenuState>> = Mutex::new(None);

type EventMap = Arc<Mutex<HashMap<MenuId, TrayEvent>>>;

pub struct TrayController {
    tray: TrayIcon,
    events: EventMap,
}

pub fn queue(state: TrayMenuState) {
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
}

fn take_pending() -> Option<TrayMenuState> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn launch_error(reason: String) -> PlatformError {
    PlatformError::Launch {
        target: "bandeja".to_string(),
        reason,
    }
}

fn hours_label(hours: u32) -> String {
    if hours == 1 {
        "Por 1 hora".to_string()
    } else {
        format!("Por {hours} horas")
    }
}

fn build_menu(state: &TrayMenuState) -> Result<(Menu, HashMap<MenuId, TrayEvent>), PlatformError> {
    let fail = |e: tray_icon::menu::Error| launch_error(e.to_string());
    let mut events = HashMap::new();
    let menu = Menu::new();

    let open = MenuItem::new(
        format!("Abrir o {}", crate::channel::app_name()),
        true,
        None,
    );
    events.insert(open.id().clone(), TrayEvent::Open);
    menu.append(&open).map_err(fail)?;

    let mute = Submenu::new("Silenciar notificações", true);
    if let Some(until) = &state.muted_until {
        let info = MenuItem::new(format!("Silenciado até {until}"), false, None);
        mute.append(&info).map_err(fail)?;
        mute.append(&PredefinedMenuItem::separator())
            .map_err(fail)?;
    }
    for hours in MUTE_HOURS {
        let item = MenuItem::new(hours_label(hours), true, None);
        events.insert(item.id().clone(), TrayEvent::Mute(hours));
        mute.append(&item).map_err(fail)?;
    }
    mute.append(&PredefinedMenuItem::separator())
        .map_err(fail)?;
    let unmute = MenuItem::new("Reativar notificações", state.muted_until.is_some(), None);
    events.insert(unmute.id().clone(), TrayEvent::Unmute);
    mute.append(&unmute).map_err(fail)?;
    menu.append(&mute).map_err(fail)?;

    let scripts = Submenu::new("Rodar script", true);
    if state.scripts.is_empty() {
        let none = MenuItem::new("Nenhum script", false, None);
        scripts.append(&none).map_err(fail)?;
    } else {
        for (id, name) in &state.scripts {
            let item = MenuItem::new(name, true, None);
            events.insert(item.id().clone(), TrayEvent::RunScript(id.clone()));
            scripts.append(&item).map_err(fail)?;
        }
    }
    menu.append(&scripts).map_err(fail)?;

    menu.append(&PredefinedMenuItem::separator())
        .map_err(fail)?;
    let quit = MenuItem::new("Sair", true, None);
    events.insert(quit.id().clone(), TrayEvent::Quit);
    menu.append(&quit).map_err(fail)?;
    Ok((menu, events))
}

impl TrayController {
    pub fn apply_pending(&mut self) {
        let Some(state) = take_pending() else {
            return;
        };
        match build_menu(&state) {
            Ok((menu, events)) => {
                *self.events.lock().unwrap_or_else(|e| e.into_inner()) = events;
                self.tray.set_menu(Some(Box::new(menu)));
            }
            Err(error) => tracing::warn!(%error, "falha ao atualizar o menu da bandeja"),
        }
    }
}

pub fn build(
    image: TrayImage,
    tooltip: &str,
    handler: TrayHandler,
) -> Result<TrayController, PlatformError> {
    let icon = Icon::from_rgba(image.rgba, image.width, image.height)
        .map_err(|e| launch_error(e.to_string()))?;
    let (menu, events) = build_menu(&TrayMenuState::default())?;
    let events: EventMap = Arc::new(Mutex::new(events));
    let handler = Arc::new(handler);
    let menu_handler = handler.clone();
    let menu_events = events.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let found = menu_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&event.id)
            .cloned();
        if let Some(found) = found {
            menu_handler(found);
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
    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip(tooltip)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| launch_error(e.to_string()))?;
    Ok(TrayController { tray, events })
}
