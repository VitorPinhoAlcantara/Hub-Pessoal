use super::{ActionHandler, Notification, PlatformError};

pub const CLOSED_ACTION: &str = "__closed";

const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 600;

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

pub fn show(
    notification: Notification,
    on_action: Option<ActionHandler>,
) -> Result<(), PlatformError> {
    let mut native = notify_rust::Notification::new();
    native
        .appname(crate::channel::app_name())
        .summary(&truncate(&notification.title, MAX_TITLE))
        .body(&truncate(&notification.body, MAX_BODY));
    #[cfg(windows)]
    native.app_id(super::windows::notification_app_id());
    for (id, label) in &notification.actions {
        native.action(id, label);
    }
    let handle = native.show().map_err(|error| PlatformError::Launch {
        target: "notificação".to_string(),
        reason: error.to_string(),
    })?;
    tracing::debug!(titulo = %notification.title, "notificação mostrada");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_text_on_char_boundary() {
        assert_eq!(truncate("ação", 10), "ação");
        assert_eq!(truncate("ãããã", 2), "ãã…");
    }
}
