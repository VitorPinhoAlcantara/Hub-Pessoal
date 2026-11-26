use std::path::PathBuf;

use hub_core::platform::clipboard;
use hub_core::whatsapp::{self, BridgeHandle, WireMessage};
use hub_ui_kit::iced::Task;
use hub_ui_kit::iced::widget::Id;
use std::sync::Arc;

use crate::chat::scroll_bottom;
use crate::media::thumbnail_bytes;
use crate::model::*;
use crate::{DRAFT_INPUT, Message, SAVED_DRAFTS, WhatsApp, apply_mentions};

async fn read_clipboard(jid: String) -> (String, Option<Pasted>) {
    let pasted = tokio::task::spawn_blocking(|| {
        let content = clipboard::read();
        content.png.map(|png| {
            let preview = thumbnail_bytes(&png, PREVIEW_SIZE);
            Pasted {
                png: Arc::new(png),
                preview,
            }
        })
    })
    .await
    .ok()
    .flatten();
    (jid, pasted)
}

impl WhatsApp {
    pub(crate) fn save_drafts(&self) {
        let list: Vec<(String, String)> = self
            .drafts
            .iter()
            .filter_map(|(jid, draft)| {
                let text = match &draft.edit {
                    Some(edit) => edit.stash.clone(),
                    None => draft.text.clone(),
                };
                (!text.trim().is_empty()).then(|| (jid.clone(), text))
            })
            .collect();
        if let Ok(mut saved) = SAVED_DRAFTS.lock() {
            *saved = list;
        }
    }

    pub(crate) fn draft_changed(&mut self, text: String) -> Task<Message> {
        let Some(draft) = self.draft_mut() else {
            return Task::none();
        };
        draft.text = text;
        let wants_group = self
            .selected
            .clone()
            .filter(|jid| whatsapp::is_group_jid(jid))
            .filter(|_| self.mention_query().is_some());
        if let (Some(jid), Some(handle)) = (wants_group, self.ctx.whatsapp.get())
            && self
                .participants
                .as_ref()
                .is_none_or(|(loaded, _)| *loaded != jid)
        {
            self.participants = Some((jid.clone(), Vec::new()));
            return Task::perform(
                async move {
                    let result = handle.participants(&jid).await;
                    (jid, result)
                },
                |(jid, result)| Message::ParticipantsLoaded(jid, result),
            );
        }
        Task::none()
    }

    pub(crate) fn mention_picked(&mut self, name: String, jid: Option<String>) -> Task<Message> {
        let Some(draft) = self.draft_mut() else {
            return Task::none();
        };
        let cut = draft
            .text
            .rfind(char::is_whitespace)
            .map_or(0, |index| index + 1);
        draft.text.truncate(cut);
        draft.text.push_str(&format!("@{name} "));
        if let Some(jid) = jid {
            draft.mentions.push((name, jid));
        }
        let id = Id::new(DRAFT_INPUT);
        Task::batch([
            hub_ui_kit::iced::widget::operation::focus(id.clone()),
            hub_ui_kit::iced::widget::operation::move_cursor_to_end(id),
        ])
    }

    pub(crate) fn emoji_picked(&mut self, emoji: String) -> Task<Message> {
        if let Some(Dialog::React(id)) = self.dialog.clone() {
            self.dialog = None;
            return self.react(id, emoji);
        }
        let Some(draft) = self.draft_mut() else {
            return Task::none();
        };
        draft.text.push_str(&emoji);
        let id = Id::new(DRAFT_INPUT);
        Task::batch([
            hub_ui_kit::iced::widget::operation::focus(id.clone()),
            hub_ui_kit::iced::widget::operation::move_cursor_to_end(id),
        ])
    }

    pub(crate) fn prepare_send(&mut self) -> Option<Outgoing> {
        let jid = self.selected.clone()?;
        if self.sending.contains(&jid) {
            return None;
        }
        let draft = self.drafts.entry(jid.clone()).or_default();
        if !draft.can_send() {
            return None;
        }
        let editing = draft.edit.take();
        let (image, voice) = if editing.is_some() {
            (None, None)
        } else {
            (draft.image.take(), draft.voice.take())
        };
        let text = draft.text.trim().to_string();
        if editing.is_some() && text.is_empty() {
            draft.edit = editing;
            return None;
        }
        let reply = if editing.is_some() {
            None
        } else if image.is_some() || voice.is_some() || !text.is_empty() {
            draft.reply.take()
        } else {
            None
        };
        let outgoing = Outgoing {
            jid: jid.clone(),
            text,
            reply,
            edit: editing,
            image,
            voice,
            files: Vec::new(),
            mentions: std::mem::take(&mut draft.mentions),
        };
        if outgoing.voice.is_none() {
            draft.text.clear();
        }
        self.sending.insert(jid);
        Some(outgoing)
    }

    pub(crate) fn send(&mut self) -> Task<Message> {
        if self.selected.is_none() || !self.is_connected() {
            return Task::none();
        }
        let Some(handle) = self.ctx.whatsapp.get() else {
            return Task::none();
        };
        let Some(outgoing) = self.prepare_send() else {
            return Task::none();
        };
        let everyone: Vec<String> = self
            .group_participants()
            .map(|list| list.iter().map(|p| p.jid.clone()).collect())
            .unwrap_or_default();
        Task::batch([dispatch(handle, outgoing, everyone), scroll_bottom()])
    }

    pub(crate) fn send_files(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let (Some(jid), Some(handle)) = (self.selected.clone(), self.ctx.whatsapp.get()) else {
            return Task::none();
        };
        if paths.is_empty() || self.sending.contains(&jid) || !self.is_connected() {
            return Task::none();
        }
        let draft = self.drafts.entry(jid.clone()).or_default();
        if draft.edit.is_some() {
            return Task::none();
        }
        let outgoing = Outgoing {
            jid: jid.clone(),
            text: draft.text.trim().to_string(),
            reply: draft.reply.take(),
            files: paths,
            ..Outgoing::default()
        };
        draft.text.clear();
        self.sending.insert(jid);
        dispatch(handle, outgoing, Vec::new())
    }

    pub(crate) fn sent(
        &mut self,
        out: Outgoing,
        results: Vec<Result<WireMessage, String>>,
    ) -> Task<Message> {
        self.sending.remove(&out.jid);
        let succeeded = results.iter().any(Result::is_ok);
        let plain_text = out.image.is_none() && out.voice.is_none() && out.files.is_empty();
        for wire in results.iter().flatten() {
            let mut wire = wire.clone();
            if plain_text {
                wire.texto = out.text.clone();
            }
            self.store(&wire);
        }
        let errors: Vec<String> = results
            .iter()
            .filter_map(|r| r.as_ref().err().cloned())
            .collect();
        if !errors.is_empty() {
            self.error = Some(format!("Não foi possível enviar: {}", errors.join("\n")));
        }
        if !succeeded {
            self.restore(out.clone());
        }
        self.reload_list();
        if self.selected.as_ref() == Some(&out.jid) {
            self.refresh_messages();
            return Task::batch([scroll_bottom(), self.pump()]);
        }
        Task::none()
    }

    pub(crate) fn restore(&mut self, out: Outgoing) {
        let draft = self.drafts.entry(out.jid).or_default();
        if let Some(edit) = out.edit {
            if draft.edit.is_none() {
                let typed = std::mem::take(&mut draft.text);
                draft.edit = Some(EditRef {
                    id: edit.id,
                    stash: if typed.trim().is_empty() {
                        edit.stash
                    } else {
                        typed
                    },
                });
                draft.text = out.text;
            }
            return;
        }
        if !out.text.is_empty() {
            if draft.text.trim().is_empty() {
                draft.text = out.text;
            } else {
                draft.text = format!("{}\n{}", out.text, draft.text);
            }
        }
        if draft.reply.is_none() {
            draft.reply = out.reply;
        }
        if draft.image.is_none() {
            draft.image = out.image;
        }
        if draft.voice.is_none() {
            draft.voice = out.voice;
        }
        draft.mentions.extend(out.mentions);
    }

    pub(crate) fn paste_shortcut(&mut self) -> Task<Message> {
        let Some(jid) = self.selected.clone() else {
            return Task::none();
        };
        if !self.is_connected() || self.dialog.is_some() || self.alias.is_some() {
            return Task::none();
        }
        Task::perform(read_clipboard(jid), |(jid, pasted)| {
            Message::PasteRead(jid, pasted)
        })
    }

    pub(crate) fn paste_read(&mut self, jid: String, pasted: Option<Pasted>) -> Task<Message> {
        let Some(pasted) = pasted else {
            return Task::none();
        };
        let draft = self.drafts.entry(jid).or_default();
        if draft.edit.is_some() || draft.voice.is_some() {
            return Task::none();
        }
        draft.image = Some(pasted);
        Self::focus_input()
    }
}

fn dispatch(handle: BridgeHandle, out: Outgoing, everyone: Vec<String>) -> Task<Message> {
    let jid = out.jid.clone();
    let reply = out.reply.as_ref().map(|r| r.id.clone());
    if let Some(edit) = out.edit.clone() {
        let text = out.text.clone();
        return Task::perform(
            async move { handle.edit(&jid, &edit.id, &text).await },
            move |result| Message::Edited(out.clone(), result),
        );
    }
    let sent = out.clone();
    Task::perform(
        async move {
            let reply = reply.as_deref();
            if let Some(voice) = &out.voice {
                let result = handle
                    .send_voice(
                        &jid,
                        &voice.path,
                        voice.seconds,
                        Some(voice.waveform.clone()),
                        reply,
                    )
                    .await;
                if result.is_ok() {
                    let _ = std::fs::remove_file(&voice.path);
                }
                return vec![result];
            }
            if let Some(image) = &out.image {
                return vec![
                    handle
                        .send_image_bytes(&jid, image.png.to_vec(), &out.text, reply)
                        .await,
                ];
            }
            if !out.files.is_empty() {
                let mut results = Vec::new();
                for (index, path) in out.files.iter().enumerate() {
                    let (caption, reply) = if index == 0 {
                        (out.text.as_str(), reply)
                    } else {
                        ("", None)
                    };
                    results.push(handle.send_file(&jid, path, caption, reply).await);
                }
                return results;
            }
            let (text, mentions) = apply_mentions(&out.text, &out.mentions, &everyone);
            vec![handle.send_text(&jid, &text, mentions, reply).await]
        },
        move |results| Message::Sent(sent.clone(), results),
    )
}
