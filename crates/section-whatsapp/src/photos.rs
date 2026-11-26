use std::path::{Path, PathBuf};

use hub_ui_kit::iced::Task;
use hub_ui_kit::iced::widget::image::Handle;

use crate::media::thumbnail;
use crate::model::*;
use crate::{Message, WhatsApp};

impl WhatsApp {
    pub(crate) fn photo_of(&self, jid: &str) -> Option<Handle> {
        match self.photos.get(jid) {
            Some(Photo::Ready(handle)) => Some(handle.clone()),
            _ => None,
        }
    }

    pub(crate) fn pump_photos(&mut self) -> Task<Message> {
        let mut candidates: Vec<(String, Option<String>)> = Vec::new();
        if let Some(conversation) = self.selected() {
            candidates.push((conversation.jid.clone(), conversation.photo.clone()));
        }
        candidates.extend(
            self.conversations()
                .into_iter()
                .take(PHOTO_CANDIDATES)
                .map(|c| (c.jid.clone(), c.photo.clone())),
        );
        let mut loading = self
            .photos
            .values()
            .filter(|state| matches!(state, Photo::Loading))
            .count();
        let mut tasks = Vec::new();
        for (jid, stored) in candidates {
            if loading >= MAX_PARALLEL_PHOTOS {
                break;
            }
            if self.photos.contains_key(&jid) {
                continue;
            }
            let cached = stored.filter(|path| Path::new(path).is_file());
            let handle = if cached.is_none() {
                match self.connected_handle() {
                    Some(handle) => Some(handle),
                    None => continue,
                }
            } else {
                None
            };
            self.photos.insert(jid.clone(), Photo::Loading);
            loading += 1;
            let target = jid.clone();
            tasks.push(Task::perform(
                async move {
                    let path = match (cached, handle) {
                        (Some(path), _) => Some(PathBuf::from(path)),
                        (None, Some(handle)) => {
                            handle.profile_picture(&target).await.ok().flatten()
                        }
                        (None, None) => None,
                    }?;
                    tokio::task::spawn_blocking(move || thumbnail(&path, AVATAR_SIZE))
                        .await
                        .ok()
                        .flatten()
                },
                move |handle| Message::PhotoReady(jid.clone(), handle),
            ));
        }
        Task::batch(tasks)
    }
}
