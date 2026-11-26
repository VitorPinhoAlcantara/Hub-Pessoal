use hub_core::whatsapp::{self, WireEdit, WireMessage, WireReaction, WireRevoke};
use hub_ui_kit::iced::Task;
use hub_ui_kit::{ContextMenu, Icon, MenuItem};

use crate::model::*;
use crate::{Message, WhatsApp};

const PREVIEW_CHARS: usize = 90;

fn snippet(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > PREVIEW_CHARS {
        let cut: String = flat.chars().take(PREVIEW_CHARS).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

impl WhatsApp {
    pub(crate) fn message_menu(&mut self, id: String) {
        let Some(message) = self.find_message(&id).cloned() else {
            return;
        };
        let now = self.now();
        let mut items = Vec::new();
        if !message.deleted {
            items.push(MenuItem::new("Responder", Message::ReplyTo(id.clone())).icon(Icon::Reply));
            items.push(
                MenuItem::new("Reagir", Message::ReactionPicker(id.clone())).icon(Icon::Smile),
            );
            if !message.text.trim().is_empty() {
                items.push(
                    MenuItem::new("Copiar texto", Message::CopyText(id.clone())).icon(Icon::Copy),
                );
            }
            items.push(
                MenuItem::new("Encaminhar", Message::ForwardOpen(id.clone())).icon(Icon::Forward),
            );
            if message.kind == whatsapp::MessageKind::Sticker
                && let Some((hash, action)) = self.sticker_of(&id)
            {
                let label = if self.is_favorite(&hash) {
                    "Tirar das favoritas"
                } else {
                    "Salvar nas favoritas"
                };
                items.push(
                    MenuItem::new(label, Message::ToggleFavorite(hash, action)).icon(Icon::Star),
                );
            }
            if can_edit(&message, now) {
                items
                    .push(MenuItem::new("Editar", Message::EditStart(id.clone())).icon(Icon::Edit));
            }
            if message.mine {
                items.push(
                    MenuItem::new("Apagar para todos", Message::RevokeAsk(id.clone()))
                        .icon(Icon::Trash)
                        .danger(),
                );
            }
        }
        items.push(
            MenuItem::new("Apagar só aqui", Message::DeleteLocalAsk(id))
                .icon(Icon::Trash)
                .danger(),
        );
        self.menu = Some(ContextMenu::at_cursor(items, Message::MenuClosed));
    }

    pub(crate) fn reply_to(&mut self, id: String) -> Task<Message> {
        let Some(message) = self.find_message(&id) else {
            return Task::none();
        };
        let reference = ReplyRef {
            id: message.id.clone(),
            label: if message.mine {
                "Você".to_string()
            } else {
                message.sender_label()
            },
            preview: snippet(&message.display_text()),
        };
        let Some(draft) = self.draft_mut() else {
            return Task::none();
        };
        if draft.edit.is_some() {
            return Task::none();
        }
        draft.reply = Some(reference);
        Self::focus_input()
    }

    pub(crate) fn react(&mut self, id: String, emoji: String) -> Task<Message> {
        let Some(jid) = self.selected.clone() else {
            return Task::none();
        };
        let Some(handle) = self.needs_connection() else {
            return Task::none();
        };
        let current = self
            .reactions
            .get(&id)
            .and_then(|list| list.iter().find(|r| r.mine))
            .map(|r| r.emoji.clone());
        let emoji = if current.as_deref() == Some(emoji.as_str()) {
            String::new()
        } else {
            emoji
        };
        Task::perform(
            async move { handle.react(&jid, &id, &emoji).await },
            Message::Reacted,
        )
    }

    pub(crate) fn reacted(&mut self, result: Result<WireReaction, String>) -> Task<Message> {
        match result {
            Ok(wire) => {
                if let Err(error) = whatsapp::apply_reaction(self.ctx.db().conn(), &wire) {
                    self.error = Some(error.to_string());
                }
                if self.selected.as_ref() == Some(&wire.jid) {
                    self.refresh_messages();
                }
            }
            Err(error) => self.error = Some(format!("Não foi possível reagir: {error}")),
        }
        Task::none()
    }

    pub(crate) fn copy_text(&mut self, id: &str) -> Task<Message> {
        match self.find_message(id) {
            Some(message) => hub_ui_kit::iced::clipboard::write(message.text.clone()),
            None => Task::none(),
        }
    }

    pub(crate) fn forward_open(&mut self, id: String) {
        let Some(origin) = self.selected.clone() else {
            return;
        };
        self.dialog = Some(Dialog::Forward(ForwardState {
            id,
            origin,
            query: String::new(),
            selected: Vec::new(),
            sending: false,
        }));
    }

    pub(crate) fn forward_targets(&self, query: &str) -> Vec<&whatsapp::Conversation> {
        self.conversations
            .iter()
            .filter(|c| c.last_at.is_some() || c.alias.is_some())
            .filter(|c| c.matches(query))
            .take(100)
            .collect()
    }

    pub(crate) fn forward_confirm(&mut self) -> Task<Message> {
        let Some(Dialog::Forward(state)) = self.dialog.clone() else {
            return Task::none();
        };
        if state.selected.is_empty() || state.sending {
            return Task::none();
        }
        let Some(handle) = self.needs_connection() else {
            return Task::none();
        };
        if let Some(Dialog::Forward(current)) = &mut self.dialog {
            current.sending = true;
        }
        Task::perform(
            async move {
                handle
                    .forward(&state.id, &state.origin, state.selected)
                    .await
            },
            Message::Forwarded,
        )
    }

    pub(crate) fn forwarded(
        &mut self,
        result: Result<Vec<Result<WireMessage, String>>, String>,
    ) -> Task<Message> {
        self.dialog = None;
        match result {
            Ok(list) => {
                let mut errors = Vec::new();
                for item in list {
                    match item {
                        Ok(wire) => self.store(&wire),
                        Err(error) => errors.push(error),
                    }
                }
                if !errors.is_empty() {
                    self.error = Some(format!(
                        "Não foi possível encaminhar: {}",
                        errors.join("\n")
                    ));
                }
                self.reload_list();
                self.refresh_messages();
            }
            Err(error) => self.error = Some(format!("Não foi possível encaminhar: {error}")),
        }
        Task::none()
    }

    pub(crate) fn edit_start(&mut self, id: String) -> Task<Message> {
        let now = self.now();
        let Some(message) = self.find_message(&id).cloned() else {
            return Task::none();
        };
        if !can_edit(&message, now) {
            self.error = Some(format!(
                "Só dá para editar mensagens suas de texto enviadas há até {EDIT_WINDOW_MINUTES} minutos."
            ));
            return Task::none();
        }
        let Some(draft) = self.draft_mut() else {
            return Task::none();
        };
        if draft.edit.is_some() {
            return Task::none();
        }
        draft.edit = Some(EditRef {
            id,
            stash: std::mem::take(&mut draft.text),
        });
        draft.text = message.text;
        draft.reply = None;
        draft.image = None;
        Self::focus_input()
    }

    pub(crate) fn cancel_edit(&mut self) {
        if let Some(draft) = self.draft_mut()
            && let Some(edit) = draft.edit.take()
        {
            draft.text = edit.stash;
        }
    }

    pub(crate) fn edited(
        &mut self,
        out: Outgoing,
        result: Result<WireEdit, String>,
    ) -> Task<Message> {
        self.sending.remove(&out.jid);
        match result {
            Ok(wire) => {
                if let Err(error) = whatsapp::apply_edit(self.ctx.db().conn(), &wire) {
                    self.error = Some(error.to_string());
                }
                if let Some(edit) = out.edit {
                    let draft = self.drafts.entry(out.jid.clone()).or_default();
                    if draft.text.trim().is_empty() {
                        draft.text = edit.stash;
                    }
                }
            }
            Err(error) => {
                self.error = Some(format!("Não foi possível editar: {error}"));
                self.restore(out.clone());
            }
        }
        if self.selected.as_ref() == Some(&out.jid) {
            self.refresh_messages();
        }
        Task::none()
    }

    pub(crate) fn revoke_confirm(&mut self, id: String) -> Task<Message> {
        self.dialog = None;
        let Some(jid) = self.selected.clone() else {
            return Task::none();
        };
        let Some(handle) = self.needs_connection() else {
            return Task::none();
        };
        Task::perform(
            async move { handle.revoke(&jid, &id).await },
            Message::Revoked,
        )
    }

    pub(crate) fn revoked(&mut self, result: Result<WireRevoke, String>) -> Task<Message> {
        match result {
            Ok(wire) => {
                if let Err(error) = whatsapp::apply_revoke(self.ctx.db().conn(), &wire) {
                    self.error = Some(error.to_string());
                }
                self.reload_list();
                if self.selected.as_ref() == Some(&wire.jid) {
                    self.refresh_messages();
                }
            }
            Err(error) => self.error = Some(format!("Não foi possível apagar: {error}")),
        }
        Task::none()
    }

    pub(crate) fn delete_local(&mut self, id: &str) {
        let Some(jid) = self.selected.clone() else {
            return;
        };
        if let Err(error) = whatsapp::delete_local(self.ctx.db().conn(), &jid, id) {
            self.error = Some(error.to_string());
            return;
        }
        if let Some(index) = self.messages.iter().position(|m| m.id == id) {
            self.messages.remove(index);
            if let Some(end) = &mut self.view_end
                && index < *end
            {
                *end -= 1;
            }
        }
        self.reactions.remove(id);
        self.reload_list();
    }

    pub(crate) fn escape(&mut self) -> Task<Message> {
        if self.menu.take().is_some() || self.dialog.take().is_some() {
            return Task::none();
        }
        if self.emoji_open {
            self.emoji_open = false;
            return Task::none();
        }
        if self.recording.is_some() {
            self.discard_recording();
            return Task::none();
        }
        if self.alias.take().is_some() {
            return Task::none();
        }
        if let Some(draft) = self.current_draft() {
            if draft.edit.is_some() {
                self.cancel_edit();
                return Task::none();
            }
            if draft.image.is_some() {
                if let Some(draft) = self.draft_mut() {
                    draft.image = None;
                }
                return Task::none();
            }
            if draft.voice.is_some() {
                self.remove_voice();
                return Task::none();
            }
            if draft.reply.is_some() {
                if let Some(draft) = self.draft_mut() {
                    draft.reply = None;
                }
                return Task::none();
            }
        }
        if self.search.is_some() {
            self.search = None;
            self.highlight = None;
            return Task::none();
        }
        if self.drawer.is_some() {
            self.drawer = None;
            return Task::none();
        }
        if self.view_end.is_some() {
            return self.back_to_recent();
        }
        Task::none()
    }
}
