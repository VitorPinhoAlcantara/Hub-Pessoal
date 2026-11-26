use std::path::PathBuf;

use hub_core::whatsapp::{self, MessageKind, Sticker, StickerSection, engine};
use hub_ui_kit::iced::Task;

use crate::media::{thumbnail, thumbnail_bytes};
use crate::model::*;
use crate::{Message, WhatsApp};

impl WhatsApp {
    pub(crate) fn pump_video_thumbs(&mut self) -> Task<Message> {
        let range = self.visible();
        let wanted: Vec<(String, String)> = self.messages[range]
            .iter()
            .filter(|m| {
                m.kind == MessageKind::Video
                    && m.downloadable
                    && !m.deleted
                    && !self.video_thumbs.contains_key(&m.id)
            })
            .map(|m| (m.jid.clone(), m.id.clone()))
            .collect();
        if wanted.is_empty() {
            return Task::none();
        }
        for (_, id) in &wanted {
            self.video_thumbs.insert(id.clone(), None);
        }
        let ctx = self.ctx.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let db = ctx.db();
                    wanted
                        .into_iter()
                        .map(|(jid, id)| {
                            let handle = engine::video_thumbnail(db.conn(), &jid, &id)
                                .and_then(|bytes| thumbnail_bytes(&bytes, PREVIEW_SIZE));
                            (id, handle)
                        })
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default()
            },
            Message::VideoThumbs,
        )
    }

    pub(crate) fn reload_favorites(&mut self) {
        match whatsapp::favorite_hashes(self.ctx.db().conn()) {
            Ok(set) => self.favorites = set,
            Err(error) => tracing::warn!(%error, "falha ao ler as figurinhas favoritas"),
        }
    }

    pub(crate) fn is_favorite(&self, hash: &str) -> bool {
        self.favorites.contains(hash)
    }

    pub(crate) fn sticker_of(&self, id: &str) -> Option<(String, Vec<u8>)> {
        let jid = self.selected.as_deref()?;
        let stored = whatsapp::raw_message(self.ctx.db().conn(), jid, id).ok()??;
        engine::sticker_action(&stored.raw)
    }

    pub(crate) fn open_stickers(&mut self) -> Task<Message> {
        self.emoji.stickers = true;
        self.emoji.query.clear();
        self.load_stickers()
    }

    pub(crate) fn pick_sticker_section(&mut self, section: StickerSection) -> Task<Message> {
        self.emoji.section = section;
        self.load_stickers()
    }

    fn load_stickers(&mut self) -> Task<Message> {
        self.stickers = None;
        let section = self.emoji.section;
        let ctx = self.ctx.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    whatsapp::stickers(ctx.db().conn(), section, STICKER_PICKER)
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
            },
            move |result| Message::StickersLoaded(section, result),
        )
    }

    pub(crate) fn stickers_loaded(
        &mut self,
        section: StickerSection,
        result: Result<Vec<Sticker>, String>,
    ) -> Task<Message> {
        if section != self.emoji.section {
            return Task::none();
        }
        let list = match result {
            Ok(list) => list,
            Err(error) => {
                self.error = Some(error);
                return Task::none();
            }
        };
        let mut tasks = Vec::new();
        for sticker in &list {
            if self.sticker_previews.contains_key(&sticker.hash) {
                continue;
            }
            self.sticker_previews.insert(sticker.hash.clone(), None);
            let local = (!sticker.file.is_empty())
                .then(|| PathBuf::from(&sticker.file))
                .filter(|path| path.is_file());
            let handle = self.connected_handle();
            let (hash, action) = (sticker.hash.clone(), sticker.action.clone());
            let key = hash.clone();
            tasks.push(Task::perform(
                async move {
                    let path = match (local, handle) {
                        (Some(path), _) => Some(path),
                        (None, Some(handle)) => handle.sticker_file(&hash, &action).await.ok(),
                        (None, None) => None,
                    }?;
                    tokio::task::spawn_blocking(move || thumbnail(&path, STICKER_PREVIEW))
                        .await
                        .ok()
                        .flatten()
                },
                move |handle| Message::StickerPreview(key.clone(), handle),
            ));
        }
        self.stickers = Some(list);
        Task::batch(tasks)
    }

    pub(crate) fn toggle_favorite(&mut self, hash: String, action: Vec<u8>) -> Task<Message> {
        let Some(handle) = self.needs_connection() else {
            return Task::none();
        };
        let favorite = !self.is_favorite(&hash);
        Task::perform(
            {
                let (hash, action) = (hash.clone(), action.clone());
                async move { handle.favorite_sticker(&hash, &action, favorite).await }
            },
            move |result| Message::FavoriteSaved(hash.clone(), action.clone(), favorite, result),
        )
    }

    pub(crate) fn favorite_saved(
        &mut self,
        hash: String,
        action: Vec<u8>,
        favorite: bool,
        result: Result<(), String>,
    ) -> Task<Message> {
        if let Err(error) = result {
            self.error = Some(error);
            return Task::none();
        }
        let at = self.now().and_utc().timestamp();
        if let Err(error) =
            whatsapp::set_favorite_sticker(self.ctx.db().conn(), &hash, &action, favorite, at)
        {
            self.error = Some(error.to_string());
        }
        self.reload_favorites();
        if self.emoji.stickers && self.emoji.section == StickerSection::Favorite {
            return self.load_stickers();
        }
        Task::none()
    }

    fn start_sticker_send(&mut self) -> Option<(String, whatsapp::BridgeHandle)> {
        let jid = self.selected.clone()?;
        let handle = self.needs_connection()?;
        if self.sending.contains(&jid) {
            return None;
        }
        self.sending.insert(jid.clone());
        self.emoji_open = false;
        Some((jid, handle))
    }

    pub(crate) fn send_sticker(&mut self, hash: String, action: Vec<u8>) -> Task<Message> {
        let Some((jid, handle)) = self.start_sticker_send() else {
            return Task::none();
        };
        let out = Outgoing {
            jid: jid.clone(),
            ..Outgoing::default()
        };
        Task::perform(
            async move { vec![handle.send_sticker(&jid, &hash, &action).await] },
            move |results| Message::Sent(out.clone(), results),
        )
    }

    pub(crate) fn send_sticker_file(&mut self, path: Option<PathBuf>) -> Task<Message> {
        let Some(path) = path else {
            return Task::none();
        };
        let Some((jid, handle)) = self.start_sticker_send() else {
            return Task::none();
        };
        let out = Outgoing {
            jid: jid.clone(),
            ..Outgoing::default()
        };
        Task::perform(
            async move { vec![handle.send_sticker_image(&jid, &path).await] },
            move |results| Message::Sent(out.clone(), results),
        )
    }
}
