use super::{ActionHandler, Notification, PlatformError};

pub const CLOSED_ACTION: &str = "__closed";

pub fn show(
    notification: Notification,
    on_action: Option<ActionHandler>,
) -> Result<(), PlatformError> {
    let mut native = notify_rust::Notification::new();
    native
        .appname(crate::channel::app_name())
        .summary(&notification.title)
        .body(&notification.body);
    for (id, label) in &notification.actions {
        native.action(id, label);
    }
    let handle = native.show().map_err(|error| PlatformError::Launch {
        target: "notificação".to_string(),
        reason: error.to_string(),
    })?;
    if let Some(on_action) = on_action {
        std::thread::Builder::new()
            .name("hub-notificacao".to_string())
            .spawn(move || {
                handle.wait_for_action(|action| {
                    let action = (action != CLOSED_ACTION).then(|| action.to_string());
                    on_action(action);
                });
            })?;
    }
    Ok(())
}
