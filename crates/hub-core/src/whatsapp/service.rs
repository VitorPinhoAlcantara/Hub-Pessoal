use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{NaiveDate, TimeDelta};
use futures::StreamExt;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use tokio::sync::oneshot;
use tokio::time::MissedTickBehavior;

use super::Conversation;
use super::engine::{self, BridgeEvent, BridgeHandle};
use super::protocol::{WireConversation, WireEdit, WireMessage, WireReaction, WireRevoke};
use crate::context::HubContext;
use crate::events::HubEvent;
use crate::platform::Notification;

pub const SESSION_ENDED: &str = "sessao_encerrada";
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(400);
pub const HISTORY_INTERVAL: Duration = Duration::from_millis(40);
pub const HISTORY_REFRESH: Duration = Duration::from_secs(1);
const WRITE_BUDGET: Duration = Duration::from_millis(25);
const WRITE_PAUSE: Duration = Duration::from_millis(10);
const HISTORY_PAGE: usize = 50;
const CHAT_INFO_BATCH: usize = 25;
pub const NOTIFY_WINDOW: TimeDelta = TimeDelta::minutes(2);
const ACTIVATION_FILE: &str = "whatsapp-ativado";
const ACTION_OPEN: &str = "abrir";
const WORKER_STACK: usize = 8 * 1024 * 1024;
const PREVIOUS_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Off,
    Starting,
    WaitingQr,
    Qr(String),
    Connected(String),
    Offline(String),
}

#[derive(Default)]
struct LinkState {
    handle: Option<BridgeHandle>,
    status: Status,
    error: Option<String>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    generation: u64,
    focus: Option<String>,
}

#[derive(Clone, Default)]
pub struct WhatsAppLink(Arc<Mutex<LinkState>>);

impl WhatsAppLink {
    fn state(&self) -> MutexGuard<'_, LinkState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set(&self, handle: Option<BridgeHandle>) {
        self.state().handle = handle;
    }

    pub fn get(&self) -> Option<BridgeHandle> {
        self.state().handle.clone()
    }

    pub fn set_focus(&self, jid: &str) {
        self.state().focus = Some(jid.to_string());
    }

    fn focus(&self) -> Option<String> {
        self.state().focus.clone()
    }

    pub fn status(&self) -> Status {
        self.state().status.clone()
    }

    pub fn take_error(&self) -> Option<String> {
        self.state().error.take()
    }

    pub fn is_running(&self) -> bool {
        self.state().stop.is_some()
    }

    pub fn is_connected(&self) -> bool {
        let state = self.state();
        matches!(state.status, Status::Connected(_)) && state.handle.is_some()
    }

    fn set_status(&self, ctx: &HubContext, status: Status) {
        self.state().status = status;
        ctx.events.publish(HubEvent::WhatsAppChanged);
    }

    fn set_error(&self, ctx: &HubContext, error: String) {
        self.state().error = Some(error);
        ctx.events.publish(HubEvent::WhatsAppChanged);
    }

    pub fn start(&self, ctx: &HubContext) {
        let (stop, stopped) = oneshot::channel();
        let generation = {
            let mut state = self.state();
            if state.stop.is_some() {
                return;
            }
            state.stop = Some(stop);
            state.status = Status::Starting;
            state.error = None;
            state.generation += 1;
            state.generation
        };
        remember_activation(ctx);
        ctx.events.publish(HubEvent::WhatsAppChanged);
        wait_previous(self);
        let ctx = ctx.clone();
        let link = self.clone();
        let spawned = std::thread::Builder::new()
            .name("hub-whatsapp".to_string())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_stack_size(WORKER_STACK)
                    .thread_name("hub-whatsapp-worker")
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        link.finish(&ctx, generation, Status::Offline(error.to_string()));
                        return;
                    }
                };
                runtime.block_on(run(ctx, link, generation, stopped));
                runtime.shutdown_timeout(Duration::from_millis(300));
            });
        match spawned {
            Ok(thread) => self.state().thread = Some(thread),
            Err(error) => {
                tracing::warn!(%error, "falha ao iniciar a thread do WhatsApp");
                self.state().stop = None;
            }
        }
    }

    pub fn shutdown(&self) {
        let (stop, thread) = {
            let mut state = self.state();
            state.handle = None;
            state.generation += 1;
            (state.stop.take(), state.thread.take())
        };
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        if let Some(thread) = thread {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    pub fn stop(&self, ctx: &HubContext) {
        let stop = {
            let mut state = self.state();
            state.handle = None;
            state.status = Status::Off;
            state.generation += 1;
            state.stop.take()
        };
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        forget_activation(ctx);
        ctx.events.publish(HubEvent::WhatsAppChanged);
    }

    pub fn restart(&self, ctx: &HubContext) {
        self.stop(ctx);
        self.start(ctx);
    }

    fn finish(&self, ctx: &HubContext, generation: u64, status: Status) {
        {
            let mut state = self.state();
            if state.generation != generation {
                return;
            }
            state.stop = None;
            state.handle = None;
            state.status = status;
        }
        ctx.events.publish(HubEvent::WhatsAppChanged);
    }
}

fn activation_file(ctx: &HubContext) -> PathBuf {
    ctx.paths.data_dir.join(ACTIVATION_FILE)
}

fn remember_activation(ctx: &HubContext) {
    let today = ctx.clock.now().date().format("%Y-%m-%d").to_string();
    if let Err(error) = std::fs::write(activation_file(ctx), today) {
        tracing::warn!(%error, "falha ao registrar a ativação do WhatsApp");
    }
}

fn forget_activation(ctx: &HubContext) {
    let _ = std::fs::remove_file(activation_file(ctx));
}

pub fn activated_today(ctx: &HubContext) -> bool {
    std::fs::read_to_string(activation_file(ctx))
        .ok()
        .and_then(|text| NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok())
        .is_some_and(|date| date == ctx.clock.now().date())
}

fn wait_previous(link: &WhatsAppLink) {
    let previous = link.state().thread.take();
    if let Some(thread) = previous {
        let deadline = std::time::Instant::now() + PREVIOUS_WAIT;
        while !thread.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

async fn run(
    ctx: HubContext,
    link: WhatsAppLink,
    generation: u64,
    mut stopped: oneshot::Receiver<()>,
) {
    let dir = ctx.paths.data_dir.clone();
    let mut events = Box::pin(engine::spawn(dir));
    let mut processor = Processor::new(ctx.clone(), link.clone());
    processor.writer = open_writer(&ctx);
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut history_ticker = tokio::time::interval(HISTORY_INTERVAL);
    history_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut engine_handle: Option<BridgeHandle> = None;
    let status = loop {
        tokio::select! {
            _ = &mut stopped => break Status::Off,
            event = events.next() => match event {
                Some(BridgeEvent::Exited(reason)) => break Status::Offline(reason),
                Some(event) => {
                    if let BridgeEvent::Ready(handle) = &event {
                        engine_handle = Some(handle.clone());
                    }
                    if let Some(handle) = processor.handle(event) {
                        let sync_ctx = ctx.clone();
                        tokio::spawn(sync_conversations(sync_ctx, handle));
                    }
                }
                None => break Status::Offline("o WhatsApp foi encerrado".to_string()),
            },
            _ = ticker.tick() => processor.flush(),
            _ = history_ticker.tick(), if processor.has_history() => processor.step_history(),
        }
    };
    processor.flush();
    if let Some(handle) = engine_handle {
        handle.shutdown().await;
    }
    processor.drain_history();
    link.finish(&ctx, generation, status);
}

async fn sync_conversations(ctx: HubContext, handle: BridgeHandle) {
    match handle.conversations().await {
        Ok(list) => {
            let saved = with_writer(&ctx, |conn| {
                let tx = begin(conn)?;
                for conversation in &list {
                    super::upsert_conversation(&tx, conversation)?;
                }
                tx.commit()
            });
            if let Err(error) = saved {
                tracing::warn!(%error, "falha ao gravar os grupos do WhatsApp");
            }
        }
        Err(error) => tracing::warn!(%error, "falha ao listar conversas do WhatsApp"),
    }
    ctx.events.publish(HubEvent::WhatsAppChanged);
    let jids = with_writer(&ctx, super::known_jids).unwrap_or_default();
    if jids.is_empty() {
        return;
    }
    match handle.chat_info(jids).await {
        Ok(infos) => {
            for batch in infos.chunks(CHAT_INFO_BATCH) {
                let saved = with_writer(&ctx, |conn| super::apply_chat_info(conn, batch));
                if let Err(error) = saved {
                    tracing::warn!(%error, "falha ao gravar informações das conversas");
                }
                tokio::time::sleep(WRITE_PAUSE).await;
            }
            ctx.events.publish(HubEvent::WhatsAppChanged);
        }
        Err(error) => tracing::warn!(%error, "falha ao consultar conversas do WhatsApp"),
    }
}

fn open_writer(ctx: &HubContext) -> Option<Connection> {
    let path = ctx.paths.database_file();
    if !path.is_file() {
        return None;
    }
    let opened = Connection::open(&path).and_then(|conn| {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(conn)
    });
    match opened {
        Ok(conn) => Some(conn),
        Err(error) => {
            tracing::warn!(%error, "falha ao abrir a conexão de gravação do WhatsApp");
            None
        }
    }
}

fn with_writer<T>(
    ctx: &HubContext,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    match open_writer(ctx) {
        Some(conn) => work(&conn),
        None => work(ctx.db().conn()),
    }
}

fn begin(conn: &Connection) -> rusqlite::Result<Transaction<'_>> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
}

#[derive(Default)]
struct ChatQueue {
    messages: Vec<WireMessage>,
    sorted: bool,
    written: usize,
    newest: i64,
}

#[derive(Default)]
pub(crate) struct HistoryQueue {
    chats: HashMap<String, ChatQueue>,
}

impl HistoryQueue {
    pub(crate) fn push(&mut self, wire: WireMessage) {
        let chat = self.chats.entry(wire.jid.clone()).or_default();
        chat.newest = chat.newest.max(wire.em);
        chat.sorted = false;
        chat.messages.push(wire);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.chats.is_empty()
    }

    pub(crate) fn next_page(&mut self, focus: Option<&str>, pinned: &[String]) -> Vec<WireMessage> {
        let focused = focus.filter(|jid| self.chats.contains_key(*jid));
        let chosen = focused.map(str::to_string).or_else(|| {
            self.chats
                .iter()
                .min_by_key(|(jid, chat)| {
                    let pin = pinned.iter().position(|p| p == *jid).unwrap_or(usize::MAX);
                    (chat.written > 0, pin, Reverse(chat.newest))
                })
                .map(|(jid, _)| jid.clone())
        });
        let Some(jid) = chosen else {
            return Vec::new();
        };
        let Some(chat) = self.chats.get_mut(&jid) else {
            return Vec::new();
        };
        if !chat.sorted {
            chat.messages.sort_by_key(|m| m.em);
            chat.sorted = true;
        }
        let start = chat.messages.len().saturating_sub(HISTORY_PAGE);
        let page = chat.messages.split_off(start);
        chat.written += page.len();
        if chat.messages.is_empty() {
            self.chats.remove(&jid);
        }
        page
    }
}

#[allow(clippy::large_enum_variant)]
enum Pending {
    Message(WireMessage),
    Edit(WireEdit),
    Revoke(WireRevoke),
    Reaction(WireReaction),
    Contact(WireConversation),
    Archived(String, bool),
    Favorite(String, bool, Vec<u8>),
}

pub struct Processor {
    ctx: HubContext,
    link: WhatsAppLink,
    writer: Option<Connection>,
    pending: VecDeque<Pending>,
    history: HistoryQueue,
    published: Instant,
}

impl Processor {
    pub fn new(ctx: HubContext, link: WhatsAppLink) -> Self {
        Self {
            ctx,
            link,
            writer: None,
            pending: VecDeque::new(),
            history: HistoryQueue::default(),
            published: Instant::now(),
        }
    }

    pub fn handle(&mut self, event: BridgeEvent) -> Option<BridgeHandle> {
        match event {
            BridgeEvent::Ready(handle) => {
                self.link.set(Some(handle));
                self.link.set_status(&self.ctx, Status::WaitingQr);
            }
            BridgeEvent::Qr(code) => self.link.set_status(&self.ctx, Status::Qr(code)),
            BridgeEvent::Connected(jid) => {
                self.link.set_status(&self.ctx, Status::Connected(jid));
                return self.link.get();
            }
            BridgeEvent::Message(wire) if wire.historico => self.history.push(wire),
            BridgeEvent::Message(wire) => self.pending.push_back(Pending::Message(wire)),
            BridgeEvent::Edit(edit) => self.pending.push_back(Pending::Edit(edit)),
            BridgeEvent::Revoke(revoke) => self.pending.push_back(Pending::Revoke(revoke)),
            BridgeEvent::Reaction(reaction) => self.pending.push_back(Pending::Reaction(reaction)),
            BridgeEvent::Contact(contact) => self.pending.push_back(Pending::Contact(contact)),
            BridgeEvent::Disconnected(reason) => {
                if reason == SESSION_ENDED {
                    self.link.set_status(&self.ctx, Status::WaitingQr);
                } else {
                    self.link.set_status(
                        &self.ctx,
                        Status::Offline("Conexão perdida. Reconectando".to_string()),
                    );
                }
            }
            BridgeEvent::Archived(jid, archived) => {
                self.pending.push_back(Pending::Archived(jid, archived));
            }
            BridgeEvent::FavoriteSticker(hash, favorite, action) => {
                self.pending
                    .push_back(Pending::Favorite(hash, favorite, action));
            }
            BridgeEvent::Error(error) => self.link.set_error(&self.ctx, error),
            BridgeEvent::Exited(reason) => {
                self.link.set(None);
                self.link.set_status(&self.ctx, Status::Offline(reason));
            }
        }
        None
    }

    pub fn has_history(&self) -> bool {
        !self.history.is_empty()
    }

    fn publish(&mut self) {
        self.published = Instant::now();
        self.ctx.events.publish(HubEvent::WhatsAppChanged);
    }

    fn write(
        &self,
        work: impl FnOnce(&Connection) -> rusqlite::Result<()>,
    ) -> rusqlite::Result<()> {
        match &self.writer {
            Some(conn) => work(conn),
            None => work(self.ctx.db().conn()),
        }
    }

    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.pending);
        let mut stored: Vec<(WireMessage, super::Message)> = Vec::new();
        while !pending.is_empty() {
            let result = self.write(|conn| {
                let tx = begin(conn)?;
                let started = Instant::now();
                while let Some(item) = pending.pop_front() {
                    if let Err(error) = apply(&tx, item, &mut stored) {
                        tracing::warn!(%error, "falha ao gravar dados do WhatsApp");
                    }
                    if started.elapsed() >= WRITE_BUDGET {
                        break;
                    }
                }
                tx.commit()
            });
            if let Err(error) = result {
                tracing::warn!(%error, "falha ao gravar mensagens do WhatsApp");
                break;
            }
            if !pending.is_empty() {
                std::thread::sleep(WRITE_PAUSE);
            }
        }
        self.pending = pending;
        self.publish();
        self.notify(&stored);
    }

    pub fn step_history(&mut self) {
        let focus = self.link.focus();
        let mut history = std::mem::take(&mut self.history);
        let mut focused = false;
        let result = self.write(|conn| {
            let tx = begin(conn)?;
            let pinned = super::pinned_jids(&tx)?;
            let started = Instant::now();
            while !history.is_empty() && started.elapsed() < WRITE_BUDGET {
                for wire in history.next_page(focus.as_deref(), &pinned) {
                    focused |= focus.as_ref() == Some(&wire.jid);
                    if let Err(error) = apply(&tx, Pending::Message(wire), &mut Vec::new()) {
                        tracing::warn!(%error, "falha ao gravar o histórico do WhatsApp");
                    }
                }
            }
            tx.commit()
        });
        self.history = history;
        if let Err(error) = result {
            tracing::warn!(%error, "falha ao gravar o histórico do WhatsApp");
            return;
        }
        let wait = if focused {
            FLUSH_INTERVAL
        } else {
            HISTORY_REFRESH
        };
        if self.history.is_empty() || self.published.elapsed() >= wait {
            self.publish();
        }
    }

    fn drain_history(&mut self) {
        let deadline = Instant::now() + PREVIOUS_WAIT;
        while self.has_history() && Instant::now() < deadline {
            self.step_history();
        }
    }

    fn notify(&self, stored: &[(WireMessage, super::Message)]) {
        let now = self.ctx.clock.now();
        let mut by_chat: BTreeMap<String, Vec<&super::Message>> = BTreeMap::new();
        for (wire, message) in stored {
            if wire.minha || wire.historico || message.at < now - NOTIFY_WINDOW {
                continue;
            }
            by_chat
                .entry(message.jid.clone())
                .or_default()
                .push(message);
        }
        for (jid, list) in by_chat {
            let conversation = super::conversation(self.ctx.db().conn(), &jid)
                .ok()
                .flatten();
            if !should_notify(&jid, conversation.as_ref()) {
                continue;
            }
            let Some(last) = list.last() else {
                continue;
            };
            let title = conversation
                .as_ref()
                .map(Conversation::title)
                .unwrap_or_else(|| {
                    super::phone_label(&jid).unwrap_or_else(|| "WhatsApp".to_string())
                });
            let text = last.display_text();
            let line = if super::is_group_jid(&jid) {
                format!("{}: {text}", last.sender_label())
            } else {
                text
            };
            let body = if list.len() > 1 {
                format!("{} mensagens novas\n{line}", list.len())
            } else {
                line
            };
            let notification = Notification {
                title,
                body,
                actions: vec![(ACTION_OPEN.to_string(), "Abrir".to_string())],
            };
            let bus = self.ctx.events.clone();
            let open_jid = jid.clone();
            self.ctx.notify(
                notification,
                Some(Box::new(move |action| {
                    if action.is_some() {
                        bus.publish(HubEvent::WhatsAppOpen(open_jid));
                    }
                })),
            );
        }
    }
}

fn apply(
    conn: &Connection,
    item: Pending,
    stored: &mut Vec<(WireMessage, super::Message)>,
) -> rusqlite::Result<()> {
    match item {
        Pending::Message(wire) => {
            let mut message = super::wire_to_message(&wire);
            if !wire.mencoes.is_empty() {
                message.text = super::resolve_mention_names(conn, &message.text, &wire.mencoes);
            }
            super::store_message(conn, &message)?;
            stored.push((wire, message));
        }
        Pending::Edit(edit) => {
            super::apply_edit(conn, &edit)?;
        }
        Pending::Revoke(revoke) => {
            super::apply_revoke(conn, &revoke)?;
        }
        Pending::Reaction(reaction) => super::apply_reaction(conn, &reaction)?,
        Pending::Contact(contact) => super::upsert_conversation(conn, &contact)?,
        Pending::Archived(jid, archived) => super::set_archived(conn, &jid, archived)?,
        Pending::Favorite(hash, favorite, action) => super::set_favorite_sticker(
            conn,
            &hash,
            &action,
            favorite,
            chrono::Utc::now().timestamp(),
        )?,
    }
    Ok(())
}

pub fn should_notify(jid: &str, conversation: Option<&Conversation>) -> bool {
    if super::is_group_jid(jid) {
        return conversation.is_some_and(|c| c.notify);
    }
    super::is_private_jid(jid)
}
