use std::collections::{HashMap, HashSet};
use std::ops::Range;

use hub_core::whatsapp::{self, HISTORY_BATCH, MESSAGES_LIMIT, Message as ChatMessage};
use hub_ui_kit::iced::Task;
use hub_ui_kit::iced::widget::Id;
use hub_ui_kit::iced::widget::scrollable::RelativeOffset;

use crate::model::*;
use crate::{Message, SEARCH_INPUT, WhatsApp};

pub fn scroll_bottom() -> Task<Message> {
    hub_ui_kit::iced::widget::operation::snap_to(
        Id::new(CHAT_SCROLL),
        RelativeOffset {
            x: None,
            y: Some(0.0),
        },
    )
}

fn flatten<T>(result: Result<Result<T, String>, tokio::task::JoinError>) -> Result<T, String> {
    result.map_err(|e| e.to_string()).and_then(|r| r)
}

impl WhatsApp {
    pub(crate) fn visible(&self) -> Range<usize> {
        visible_range(self.messages.len(), self.view_end, self.shown)
    }

    pub(crate) fn current_draft(&self) -> Option<&Draft> {
        self.selected.as_ref().and_then(|jid| self.drafts.get(jid))
    }

    pub(crate) fn draft_mut(&mut self) -> Option<&mut Draft> {
        let jid = self.selected.clone()?;
        Some(self.drafts.entry(jid).or_default())
    }

    pub(crate) fn select(&mut self, jid: String) -> Task<Message> {
        if self.selected.as_ref() == Some(&jid) {
            return Task::none();
        }
        self.discard_recording();
        self.ctx.whatsapp.set_focus(&jid);
        self.selected = Some(jid);
        self.alias = None;
        self.search = None;
        self.highlight = None;
        self.history_wait = None;
        self.history_notice = None;
        self.dialog = None;
        self.emoji_open = false;
        self.menu = None;
        self.load_messages();
        Task::batch([scroll_bottom(), self.pump(), self.drawer_task()])
    }

    fn load_messages(&mut self) {
        self.view_end = None;
        self.shown = WINDOW_SIZE;
        match self.selected.clone() {
            Some(jid) => {
                self.messages = whatsapp::messages(self.ctx.db().conn(), &jid, MESSAGES_LIMIT)
                    .unwrap_or_default();
                self.load_reactions(&jid);
            }
            None => {
                self.messages.clear();
                self.reactions.clear();
            }
        }
    }

    pub(crate) fn refresh_messages(&mut self) {
        let Some(jid) = self.selected.clone() else {
            return;
        };
        let fresh =
            whatsapp::messages(self.ctx.db().conn(), &jid, MESSAGES_LIMIT).unwrap_or_default();
        let current = std::mem::take(&mut self.messages);
        let (merged, kept) = merge_messages(current, fresh);
        self.messages = merged;
        if !kept {
            self.view_end = None;
            self.shown = WINDOW_SIZE;
        }
        self.load_reactions(&jid);
    }

    fn load_reactions(&mut self, jid: &str) {
        let list = whatsapp::reactions(self.ctx.db().conn(), jid).unwrap_or_default();
        let mut grouped: HashMap<String, Vec<whatsapp::Reaction>> = HashMap::new();
        for reaction in list {
            grouped
                .entry(reaction.message_id.clone())
                .or_default()
                .push(reaction);
        }
        self.reactions = grouped;
    }

    pub(crate) fn load_older(&mut self) -> Task<Message> {
        if self.selected.is_none() || self.history_wait.is_some() {
            return Task::none();
        }
        self.highlight = None;
        self.history_notice = None;
        let range = self.visible();
        if range.start > 0 {
            self.shown += WINDOW_STEP;
            return Task::none();
        }
        self.fetch_older()
    }

    pub(crate) fn fetch_older(&mut self) -> Task<Message> {
        let (Some(jid), Some(oldest)) = (self.selected.clone(), self.messages.first().cloned())
        else {
            return self.request_phone_history();
        };
        let ctx = self.ctx.clone();
        let target = jid.clone();
        Task::perform(
            async move {
                flatten(
                    tokio::task::spawn_blocking(move || {
                        whatsapp::messages_before(
                            ctx.db().conn(),
                            &target,
                            oldest.at,
                            &oldest.id,
                            OLDER_BATCH,
                        )
                        .map_err(|e| e.to_string())
                    })
                    .await,
                )
            },
            move |result| Message::OlderLoaded(jid.clone(), result),
        )
    }

    pub(crate) fn older_loaded(
        &mut self,
        jid: String,
        result: Result<Vec<ChatMessage>, String>,
    ) -> Task<Message> {
        if self.selected.as_ref() != Some(&jid) {
            return Task::none();
        }
        let list = match result {
            Ok(list) => list,
            Err(error) => {
                self.history_wait = None;
                self.error = Some(error);
                return Task::none();
            }
        };
        let known: HashSet<&str> = self.messages.iter().map(|m| m.id.as_str()).collect();
        let fresh: Vec<ChatMessage> = list
            .into_iter()
            .filter(|m| !known.contains(m.id.as_str()))
            .collect();
        if fresh.is_empty() {
            if self.history_wait.is_some() {
                return Task::none();
            }
            return self.request_phone_history();
        }
        self.history_wait = None;
        self.history_notice = None;
        self.prepend(fresh);
        self.load_reactions(&jid);
        self.pump()
    }

    fn prepend(&mut self, mut older: Vec<ChatMessage>) {
        let added = older.len();
        older.append(&mut self.messages);
        self.messages = older;
        self.shown += added;
        if let Some(end) = &mut self.view_end {
            *end += added;
        }
    }

    fn request_phone_history(&mut self) -> Task<Message> {
        let Some(jid) = self.selected.clone() else {
            return Task::none();
        };
        let Some(handle) = self.connected_handle() else {
            self.history_notice = Some(
                "Conecte o WhatsApp para pedir mensagens mais antigas ao celular.".to_string(),
            );
            return Task::none();
        };
        self.history_notice = Some("Pedindo mensagens mais antigas ao celular".to_string());
        let target = jid.clone();
        Task::perform(
            async move { handle.request_history(&target, HISTORY_BATCH).await },
            move |result| Message::HistoryRequested(jid.clone(), result),
        )
    }

    pub(crate) fn history_requested(
        &mut self,
        jid: String,
        result: Result<(), String>,
    ) -> Task<Message> {
        if self.selected.as_ref() != Some(&jid) {
            return Task::none();
        }
        match result {
            Ok(()) => {
                self.history_generation += 1;
                let generation = self.history_generation;
                self.history_wait = Some(HistoryWait {
                    jid: jid.clone(),
                    generation,
                });
                Task::perform(tokio::time::sleep(HISTORY_TIMEOUT), move |_| {
                    Message::HistoryTimeout(jid.clone(), generation)
                })
            }
            Err(error) => {
                self.history_notice = Some(error);
                Task::none()
            }
        }
    }

    pub(crate) fn history_timeout(&mut self, jid: &str, generation: u64) {
        let waiting = self
            .history_wait
            .as_ref()
            .is_some_and(|w| w.jid == jid && w.generation == generation);
        if waiting {
            self.history_wait = None;
            self.history_notice = Some(
                "O celular não devolveu mensagens mais antigas. Tente de novo mais tarde."
                    .to_string(),
            );
        }
    }

    fn focus_on(&mut self, index: usize) {
        let len = self.messages.len();
        let end = (index + 1 + JUMP_TAIL).min(len);
        self.view_end = (end < len).then_some(end);
        self.shown = self.shown.max(end - index + 10);
    }

    pub(crate) fn jump_to(&mut self, jid: String, id: String) -> Task<Message> {
        let switch = self.select(jid.clone());
        if self.selected.as_ref() != Some(&jid) {
            return switch;
        }
        self.highlight = Some(id.clone());
        if let Some(index) = self.messages.iter().position(|m| m.id == id) {
            self.focus_on(index);
            return Task::batch([switch, scroll_bottom(), self.pump()]);
        }
        let Some(oldest) = self.messages.first().cloned() else {
            return switch;
        };
        let ctx = self.ctx.clone();
        let target = id.clone();
        let chat = jid.clone();
        let load = Task::perform(
            async move {
                flatten(
                    tokio::task::spawn_blocking(move || {
                        let db = ctx.db();
                        let conn = db.conn();
                        let (mut at, mut anchor) = (oldest.at, oldest.id);
                        let mut collected: Vec<ChatMessage> = Vec::new();
                        loop {
                            let batch =
                                whatsapp::messages_before(conn, &chat, at, &anchor, JUMP_BATCH)
                                    .map_err(|e| e.to_string())?;
                            let Some(first) = batch.first() else {
                                break;
                            };
                            at = first.at;
                            anchor = first.id.clone();
                            let found = batch.iter().any(|m| m.id == target);
                            let mut combined = batch;
                            combined.append(&mut collected);
                            collected = combined;
                            if found || collected.len() >= JUMP_LIMIT {
                                break;
                            }
                        }
                        Ok(collected)
                    })
                    .await,
                )
            },
            move |result| Message::JumpLoaded(jid.clone(), id.clone(), result),
        );
        Task::batch([switch, load])
    }

    pub(crate) fn jump_loaded(
        &mut self,
        jid: String,
        id: String,
        result: Result<Vec<ChatMessage>, String>,
    ) -> Task<Message> {
        if self.selected.as_ref() != Some(&jid) || self.highlight.as_ref() != Some(&id) {
            return Task::none();
        }
        let list = match result {
            Ok(list) => list,
            Err(error) => {
                self.error = Some(error);
                return Task::none();
            }
        };
        let known: HashSet<&str> = self.messages.iter().map(|m| m.id.as_str()).collect();
        let fresh: Vec<ChatMessage> = list
            .into_iter()
            .filter(|m| !known.contains(m.id.as_str()))
            .collect();
        if !fresh.is_empty() {
            self.prepend(fresh);
            self.load_reactions(&jid);
        }
        match self.messages.iter().position(|m| m.id == id) {
            Some(index) => {
                self.focus_on(index);
                Task::batch([scroll_bottom(), self.pump()])
            }
            None => {
                self.highlight = None;
                self.error = Some("Não foi possível carregar essa mensagem.".to_string());
                Task::none()
            }
        }
    }

    pub(crate) fn back_to_recent(&mut self) -> Task<Message> {
        self.view_end = None;
        self.shown = WINDOW_SIZE;
        self.highlight = None;
        Task::batch([scroll_bottom(), self.pump()])
    }

    pub(crate) fn search_toggle(&mut self) -> Task<Message> {
        if self.search.is_some() {
            self.search = None;
            self.highlight = None;
            return Task::none();
        }
        self.search = Some(SearchState::default());
        hub_ui_kit::iced::widget::operation::focus(Id::new(SEARCH_INPUT))
    }

    pub(crate) fn search_changed(&mut self, query: String) -> Task<Message> {
        let Some(jid) = self.selected.clone() else {
            return Task::none();
        };
        self.search_seq += 1;
        let seq = self.search_seq;
        let Some(state) = &mut self.search else {
            return Task::none();
        };
        state.query = query.clone();
        state.seq = seq;
        state.results.clear();
        state.index = 0;
        let trimmed = query.trim().to_string();
        if trimmed.is_empty() {
            state.loading = false;
            return Task::none();
        }
        state.loading = true;
        let ctx = self.ctx.clone();
        Task::perform(
            async move {
                flatten(
                    tokio::task::spawn_blocking(move || {
                        whatsapp::search_messages(
                            ctx.db().conn(),
                            &trimmed,
                            Some(&jid),
                            SEARCH_LIMIT,
                        )
                        .map_err(|e| e.to_string())
                    })
                    .await,
                )
            },
            move |result| Message::SearchResults(seq, result),
        )
    }

    pub(crate) fn search_results(
        &mut self,
        seq: u64,
        result: Result<Vec<ChatMessage>, String>,
    ) -> Task<Message> {
        let Some(state) = &mut self.search else {
            return Task::none();
        };
        if state.seq != seq {
            return Task::none();
        }
        state.loading = false;
        match result {
            Ok(list) => state.results = list,
            Err(error) => {
                self.error = Some(error);
                return Task::none();
            }
        }
        state.index = 0;
        self.jump_to_result()
    }

    fn jump_to_result(&mut self) -> Task<Message> {
        let (Some(jid), Some(state)) = (self.selected.clone(), &self.search) else {
            return Task::none();
        };
        match state.results.get(state.index) {
            Some(hit) => {
                let id = hit.id.clone();
                self.jump_to(jid, id)
            }
            None => Task::none(),
        }
    }

    pub(crate) fn search_step(&mut self, older: bool) -> Task<Message> {
        let Some(state) = &mut self.search else {
            return Task::none();
        };
        if state.results.is_empty() {
            return Task::none();
        }
        let last = state.results.len() - 1;
        state.index = if older {
            (state.index + 1).min(last)
        } else {
            state.index.saturating_sub(1)
        };
        self.jump_to_result()
    }

    pub(crate) fn drawer_toggle(&mut self) -> Task<Message> {
        if self.drawer.is_some() {
            self.drawer = None;
            return Task::none();
        }
        self.drawer = Some(Drawer {
            tab: DrawerTab::Media,
            items: Vec::new(),
            loading: false,
        });
        self.drawer_task()
    }

    pub(crate) fn drawer_task(&mut self) -> Task<Message> {
        let (Some(jid), Some(drawer)) = (self.selected.clone(), &mut self.drawer) else {
            return Task::none();
        };
        drawer.loading = true;
        let ctx = self.ctx.clone();
        let target = jid.clone();
        Task::perform(
            async move {
                flatten(
                    tokio::task::spawn_blocking(move || {
                        whatsapp::messages(ctx.db().conn(), &target, DRAWER_LOAD)
                            .map_err(|e| e.to_string())
                    })
                    .await,
                )
            },
            move |result| Message::DrawerLoaded(jid.clone(), result),
        )
    }

    pub(crate) fn drawer_loaded(&mut self, jid: &str, result: Result<Vec<ChatMessage>, String>) {
        let Some(drawer) = &mut self.drawer else {
            return;
        };
        drawer.loading = false;
        if self.selected.as_deref() != Some(jid) {
            return;
        }
        drawer.items = result
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                [DrawerTab::Media, DrawerTab::Documents, DrawerTab::Audios]
                    .iter()
                    .any(|tab| tab.accepts(m))
            })
            .rev()
            .collect();
    }
}
