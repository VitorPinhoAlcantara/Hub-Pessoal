use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hub_core::audio::{Playback, Recorder};
use hub_ui_kit::iced::Task;

use crate::model::*;
use crate::{Message, WhatsApp};

pub struct Recording {
    pub jid: String,
    pub recorder: Recorder,
}

pub struct Player {
    pub id: String,
    pub playback: Playback,
    pub seeking: Option<f32>,
}

#[derive(Clone)]
pub struct ReadyPlayback(Arc<Mutex<Option<Playback>>>);

impl std::fmt::Debug for ReadyPlayback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadyPlayback")
    }
}

pub struct Progress {
    pub fraction: f32,
    pub paused: bool,
    pub position: Duration,
    pub total: Duration,
}

impl WhatsApp {
    pub(crate) fn is_ticking(&self) -> bool {
        self.recording.is_some()
            || self
                .player
                .as_ref()
                .is_some_and(|player| !player.playback.is_paused())
    }

    pub(crate) fn recording_elapsed(&self) -> Option<Duration> {
        self.recording
            .as_ref()
            .filter(|r| Some(&r.jid) == self.selected.as_ref())
            .map(|r| r.recorder.elapsed())
    }

    pub(crate) fn start_recording(&mut self) {
        let Some(jid) = self.selected.clone() else {
            return;
        };
        if self.recording.is_some() || !self.is_connected() || self.sending.contains(&jid) {
            return;
        }
        let blocked = self
            .current_draft()
            .is_some_and(|d| d.edit.is_some() || d.voice.is_some() || d.image.is_some());
        if blocked {
            return;
        }
        let dir = self.ctx.paths.data_dir.join("whatsapp-voz");
        if let Err(error) = std::fs::create_dir_all(&dir) {
            self.error = Some(format!("Não foi possível preparar a gravação: {error}"));
            return;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        match Recorder::start(dir.join(format!("voz-{stamp}.ogg"))) {
            Ok(recorder) => self.recording = Some(Recording { jid, recorder }),
            Err(error) => self.error = Some(format!("Não foi possível gravar: {error}")),
        }
    }

    pub(crate) fn discard_recording(&mut self) {
        if let Some(recording) = self.recording.take() {
            recording.recorder.cancel();
        }
    }

    pub(crate) fn stop_recording(&mut self) -> Task<Message> {
        let Some(recording) = self.recording.take() else {
            return Task::none();
        };
        let jid = recording.jid.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    recording
                        .recorder
                        .finish()
                        .map(|note| Voice {
                            path: note.path,
                            seconds: note.seconds,
                            waveform: note.waveform,
                        })
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result)
            },
            move |result| Message::VoiceReady(jid.clone(), result),
        )
    }

    pub(crate) fn voice_ready(
        &mut self,
        jid: String,
        result: Result<Voice, String>,
    ) -> Task<Message> {
        match result {
            Ok(voice) => {
                self.drafts.entry(jid.clone()).or_default().voice = Some(voice);
                if self.selected.as_ref() == Some(&jid) {
                    return self.send();
                }
                Task::none()
            }
            Err(error) => {
                self.error = Some(format!("Não foi possível gravar: {error}"));
                Task::none()
            }
        }
    }

    pub(crate) fn remove_voice(&mut self) {
        if let Some(voice) = self.draft_mut().and_then(|d| d.voice.take()) {
            let _ = std::fs::remove_file(voice.path);
        }
    }

    pub(crate) fn tick(&mut self) {
        let failure = self.recording.as_ref().and_then(|r| r.recorder.error());
        if let Some(error) = failure {
            self.discard_recording();
            self.error = Some(format!("A gravação foi interrompida: {error}"));
        }
        if self.player.as_ref().is_some_and(|p| p.playback.finished()) {
            self.player = None;
        }
    }

    pub(crate) fn start_playback(&mut self, id: String, path: &Path) -> Task<Message> {
        self.player = None;
        self.loading_audio = Some(id.clone());
        let path = path.to_path_buf();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || Playback::start(&path))
                    .await
                    .map_err(|error| error.to_string())?
                    .map(|playback| ReadyPlayback(Arc::new(Mutex::new(Some(playback)))))
                    .map_err(|error| error.to_string())
            },
            move |result| Message::PlaybackReady(id.clone(), result),
        )
    }

    pub(crate) fn playback_ready(&mut self, id: String, result: Result<ReadyPlayback, String>) {
        if self.loading_audio.as_ref() != Some(&id) {
            return;
        }
        self.loading_audio = None;
        match result {
            Ok(ready) => {
                let taken = ready.0.lock().unwrap_or_else(|e| e.into_inner()).take();
                let Some(playback) = taken else {
                    return;
                };
                playback.set_speed(SPEEDS[self.speed]);
                self.player = Some(Player {
                    id,
                    playback,
                    seeking: None,
                });
            }
            Err(error) => self.error = Some(format!("Não foi possível tocar o áudio: {error}")),
        }
    }

    pub(crate) fn toggle_audio(&mut self, id: String) -> Task<Message> {
        if let Some(player) = &self.player
            && player.id == id
        {
            if player.playback.is_paused() {
                player.playback.resume();
            } else {
                player.playback.pause();
            }
            return Task::none();
        }
        if self.loading_audio.as_ref() == Some(&id) {
            return Task::none();
        }
        self.request_media(id, Intent::Play)
    }

    pub(crate) fn commit_seek(&mut self) {
        let Some(player) = &mut self.player else {
            return;
        };
        if let Some(fraction) = player.seeking.take() {
            let total = player.playback.duration();
            player
                .playback
                .seek(total.mul_f32(fraction.clamp(0.0, 1.0)));
        }
    }

    pub(crate) fn cycle_speed(&mut self) {
        self.speed = (self.speed + 1) % SPEEDS.len();
        if let Some(player) = &self.player {
            player.playback.set_speed(SPEEDS[self.speed]);
        }
    }

    pub(crate) fn progress_of(&self, id: &str, fallback_seconds: u32) -> Option<Progress> {
        let player = self.player.as_ref().filter(|p| p.id == id)?;
        let mut total = player.playback.duration();
        if total.is_zero() {
            total = Duration::from_secs(fallback_seconds as u64);
        }
        let position = player.playback.position();
        let fraction = player.seeking.unwrap_or_else(|| {
            if total.is_zero() {
                0.0
            } else {
                (position.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
            }
        });
        Some(Progress {
            fraction,
            paused: player.playback.is_paused(),
            position,
            total,
        })
    }
}
