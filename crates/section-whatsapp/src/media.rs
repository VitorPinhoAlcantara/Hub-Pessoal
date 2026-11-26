use std::collections::HashSet;
use std::path::{Path, PathBuf};

use hub_core::whatsapp::{Message as ChatMessage, MessageKind};
use hub_ui_kit::iced::Task;
use hub_ui_kit::iced::widget::image::Handle;

use crate::model::*;
use crate::{Message, WhatsApp};

const MAX_DECODE_BYTES: u64 = 60 * 1024 * 1024;

fn rgba_handle(decoded: image::DynamicImage, max: u32) -> Option<Handle> {
    let fitted = if decoded.width() > max || decoded.height() > max {
        decoded.thumbnail(max, max)
    } else {
        decoded
    };
    let rgba = fitted.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(Handle::from_rgba(width, height, rgba.into_raw()))
}

pub fn thumbnail(path: &Path, max: u32) -> Option<Handle> {
    if std::fs::metadata(path).ok()?.len() > MAX_DECODE_BYTES {
        return None;
    }
    let decoded = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    rgba_handle(decoded, max)
}

pub fn thumbnail_bytes(bytes: &[u8], max: u32) -> Option<Handle> {
    rgba_handle(image::load_from_memory(bytes).ok()?, max)
}

pub fn can_fetch(message: &ChatMessage) -> bool {
    message.kind.is_media()
        && !message.deleted
        && (message.downloadable || !message.file.is_empty())
}

fn wants_preview(kind: MessageKind) -> bool {
    matches!(kind, MessageKind::Image | MessageKind::Sticker)
}

impl WhatsApp {
    pub(crate) fn display_name_of(&self, id: &str, path: &Path) -> String {
        let name = self
            .find_message(id)
            .map(|m| m.file_name.clone())
            .unwrap_or_default();
        if name.is_empty() {
            path.to_string_lossy().into_owned()
        } else {
            name
        }
    }

    pub(crate) fn open_safely(&mut self, path: &Path, name: &str) {
        let dangerous = is_dangerous(name) || is_dangerous(&path.to_string_lossy());
        let result = if dangerous {
            self.reveal(path)
        } else {
            self.ctx
                .platform
                .open_path(path)
                .map_err(|error| error.to_string())
        };
        if let Err(error) = result {
            self.error = Some(format!("Não foi possível abrir o arquivo: {error}"));
        }
    }

    fn reveal(&self, path: &Path) -> Result<(), String> {
        if self.ctx.platform.reveal_path(path).is_ok() {
            return Ok(());
        }
        match path.parent() {
            Some(folder) => self
                .ctx
                .platform
                .open_path(folder)
                .map_err(|error| error.to_string()),
            None => Err("pasta não encontrada".to_string()),
        }
    }

    pub(crate) fn reveal_media(&mut self, id: &str) {
        let Some(Media::Ready(_, path)) = self.media.get(id) else {
            return;
        };
        if let Err(error) = self.reveal(&path.clone()) {
            self.error = Some(format!("Não foi possível abrir a pasta: {error}"));
        }
    }

    fn finish_intent(&mut self, id: &str, path: &Path, intent: Intent) -> Task<Message> {
        match intent {
            Intent::Show => Task::none(),
            Intent::Open => {
                let name = self.display_name_of(id, path);
                self.open_safely(path, &name);
                Task::none()
            }
            Intent::Play => self.start_playback(id.to_string(), path),
        }
    }

    pub(crate) fn request_media(&mut self, id: String, intent: Intent) -> Task<Message> {
        self.start_download(id, intent, false)
    }

    fn start_download(&mut self, id: String, intent: Intent, auto: bool) -> Task<Message> {
        let Some(message) = self.find_message(&id).cloned() else {
            return Task::none();
        };
        match self.media.get(&id) {
            Some(Media::Loading) => {
                if intent != Intent::Show {
                    self.intents.insert(id, intent);
                }
                return Task::none();
            }
            Some(Media::Ready(_, path)) => {
                let path = path.clone();
                return self.finish_intent(&id, &path, intent);
            }
            _ => {}
        }
        let Some(jid) = self.selected.clone() else {
            return Task::none();
        };
        if !can_fetch(&message) {
            return Task::none();
        }
        let local = (!message.file.is_empty())
            .then(|| PathBuf::from(&message.file))
            .filter(|path| path.is_file());
        let handle = if local.is_none() {
            match self.connected_handle() {
                Some(handle) => Some(handle),
                None if auto => return Task::none(),
                None => {
                    self.media.insert(
                        id,
                        Media::Failed("Conecte o WhatsApp para baixar o arquivo.".to_string()),
                    );
                    return Task::none();
                }
            }
        } else {
            None
        };
        self.media.insert(id.clone(), Media::Loading);
        if intent != Intent::Show {
            self.intents.insert(id.clone(), intent);
        }
        let preview = wants_preview(message.kind);
        let key = id.clone();
        Task::perform(
            async move {
                let path = match (local, handle) {
                    (Some(path), _) => path,
                    (None, Some(handle)) => handle.download_media(&jid, &id).await?,
                    (None, None) => return Err("Arquivo indisponível".to_string()),
                };
                let preview = if preview {
                    let source = path.clone();
                    tokio::task::spawn_blocking(move || thumbnail(&source, PREVIEW_SIZE))
                        .await
                        .ok()
                        .flatten()
                } else {
                    None
                };
                Ok(MediaFile { path, preview })
            },
            move |result| Message::MediaLoaded(key.clone(), result),
        )
    }

    pub(crate) fn media_loaded(
        &mut self,
        id: String,
        result: Result<MediaFile, String>,
    ) -> Task<Message> {
        let intent = self.intents.remove(&id);
        let mut follow_up = Task::none();
        match result {
            Ok(file) => {
                if file.preview.is_some() {
                    self.preview_order.retain(|known| *known != id);
                    self.preview_order.push(id.clone());
                    self.evict_previews();
                }
                self.media
                    .insert(id.clone(), Media::Ready(file.preview, file.path.clone()));
                if let Some(intent) = intent {
                    follow_up = self.finish_intent(&id, &file.path, intent);
                }
            }
            Err(error) => {
                self.media.insert(id, Media::Failed(error));
            }
        }
        Task::batch([follow_up, self.pump_media()])
    }

    pub(crate) fn drop_hidden_previews(&mut self) {
        let range = self.visible();
        let keep: HashSet<String> = self.messages[range].iter().map(|m| m.id.clone()).collect();
        self.preview_order.retain(|id| keep.contains(id));
        self.media
            .retain(|id, state| keep.contains(id) || !matches!(state, Media::Ready(Some(_), _)));
    }

    fn evict_previews(&mut self) {
        let range = self.visible();
        let keep: HashSet<String> = self.messages[range].iter().map(|m| m.id.clone()).collect();
        let dropped = trim_previews(&mut self.preview_order, &keep, MAX_PREVIEWS);
        for id in dropped {
            if matches!(self.media.get(&id), Some(Media::Ready(Some(_), _))) {
                self.media.remove(&id);
            }
        }
    }

    pub(crate) fn pump_media(&mut self) -> Task<Message> {
        let range = self.visible();
        let candidates: Vec<String> = self.messages[range]
            .iter()
            .rev()
            .filter(|m| {
                wants_preview(m.kind) && can_fetch(m) && (m.size == 0 || m.size <= AUTO_MEDIA_LIMIT)
            })
            .take(AUTO_MEDIA_RECENT)
            .map(|m| m.id.clone())
            .collect();
        let loading = self
            .media
            .values()
            .filter(|state| matches!(state, Media::Loading))
            .count();
        let mut free = MAX_PARALLEL_DOWNLOADS.saturating_sub(loading);
        let mut tasks = Vec::new();
        for id in candidates {
            if free == 0 {
                break;
            }
            if self.media.contains_key(&id) {
                continue;
            }
            let task = self.start_download(id.clone(), Intent::Show, true);
            if matches!(self.media.get(&id), Some(Media::Loading)) {
                free -= 1;
                tasks.push(task);
            }
        }
        Task::batch(tasks)
    }
}
