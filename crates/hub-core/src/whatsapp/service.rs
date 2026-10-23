use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{NaiveDate, TimeDelta};
use futures::StreamExt;
use tokio::sync::oneshot;

use super::bridge::{self, BridgeEvent, BridgeHandle};
use super::protocol::{Request, WireMessage};
use super::{Conversation, sidecar_command};
use crate::context::HubContext;
use crate::events::HubEvent;
use crate::platform::Notification;

pub const SESSION_ENDED: &str = "sessao_encerrada";
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(400);
pub const NOTIFY_WINDOW: TimeDelta = TimeDelta::minutes(2);
const ACTIVATION_FILE: &str = "whatsapp-ativado";
const ACTION_OPEN: &str = "abrir";

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
        let ctx = ctx.clone();
        let link = self.clone();
        let spawned = std::thread::Builder::new()
            .name("hub-whatsapp".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
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

async fn run(
    ctx: HubContext,
    link: WhatsAppLink,
    generation: u64,
    mut stopped: oneshot::Receiver<()>,
) {
    let dir = ctx.paths.data_dir.clone();
    let mut events = Box::pin(bridge::spawn(sidecar_command(&dir), dir));
    let mut processor = Processor::new(ctx.clone(), link.clone());
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    let status = loop {
        tokio::select! {
            _ = &mut stopped => break Status::Off,
            event = events.next() => match event {
                Some(BridgeEvent::Exited(reason)) => break Status::Offline(reason),
                Some(event) => {
                    if let Some(handle) = processor.handle(event) {
                        let sync_ctx = ctx.clone();
                        tokio::spawn(sync_conversations(sync_ctx, handle));
                    }
                }
                None => break Status::Offline("o WhatsApp foi encerrado".to_string()),
            },
            _ = ticker.tick() => processor.flush(),
        }
    };
    processor.flush();
    link.finish(&ctx, generation, status);
}

async fn sync_conversations(ctx: HubContext, handle: BridgeHandle) {
    match handle.conversations().await {
        Ok(list) => {
            let db = ctx.db();
            for conversation in &list {
                let _ = super::upsert_conversation(db.conn(), conversation);
            }
        }
        Err(error) => tracing::warn!(%error, "falha ao listar conversas do WhatsApp"),
    }
    ctx.events.publish(HubEvent::WhatsAppChanged);
    let jids = super::known_jids(ctx.db().conn()).unwrap_or_default();
    if jids.is_empty() {
        return;
    }
    match handle.chat_info(jids).await {
        Ok(infos) => {
            if let Err(error) = super::apply_chat_info(ctx.db().conn(), &infos) {
                tracing::warn!(%error, "falha ao gravar informações das conversas");
            }
            ctx.events.publish(HubEvent::WhatsAppChanged);
        }
        Err(error) => tracing::warn!(%error, "falha ao consultar conversas do WhatsApp"),
    }
}

pub struct Processor {
    ctx: HubContext,
    link: WhatsAppLink,
    pending: Vec<WireMessage>,
}

impl Processor {
    pub fn new(ctx: HubContext, link: WhatsAppLink) -> Self {
        Self {
            ctx,
            link,
            pending: Vec::new(),
        }
    }

    pub fn handle(&mut self, event: BridgeEvent) -> Option<BridgeHandle> {
        match event {
            BridgeEvent::Ready(handle) => {
                handle.send(Request::Login);
                self.link.set(Some(handle));
                self.link.set_status(&self.ctx, Status::WaitingQr);
            }
            BridgeEvent::Qr(code) => self.link.set_status(&self.ctx, Status::Qr(code)),
            BridgeEvent::Connected(jid) => {
                self.link.set_status(&self.ctx, Status::Connected(jid));
                return self.link.get();
            }
            BridgeEvent::Message(wire) => self.pending.push(wire),
            BridgeEvent::Disconnected(reason) => {
                if reason == SESSION_ENDED {
                    self.link.set_status(&self.ctx, Status::WaitingQr);
                    if let Some(handle) = self.link.get() {
                        handle.send(Request::Login);
                    }
                } else {
                    self.link.set_status(
                        &self.ctx,
                        Status::Offline(format!("Conexão perdida ({reason})")),
                    );
                }
            }
            BridgeEvent::Archived(jid, archived) => {
                if let Err(error) = super::set_archived(self.ctx.db().conn(), &jid, archived) {
                    tracing::warn!(%error, "falha ao marcar conversa arquivada");
                }
                self.ctx.events.publish(HubEvent::WhatsAppChanged);
            }
            BridgeEvent::Error(error) => self.link.set_error(&self.ctx, error),
            BridgeEvent::Exited(reason) => {
                self.link.set(None);
                self.link.set_status(&self.ctx, Status::Offline(reason));
            }
        }
        None
    }

    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        let messages: Vec<super::Message> = pending.iter().map(super::wire_to_message).collect();
        if let Err(error) = super::store_messages(self.ctx.db().conn(), &messages) {
            tracing::warn!(%error, "falha ao gravar mensagens do WhatsApp");
        }
        self.ctx.events.publish(HubEvent::WhatsAppChanged);
        self.notify(&pending, &messages);
    }

    fn notify(&self, pending: &[WireMessage], messages: &[super::Message]) {
        let now = self.ctx.clock.now();
        let mut by_chat: BTreeMap<String, Vec<&super::Message>> = BTreeMap::new();
        for (wire, message) in pending.iter().zip(messages) {
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

pub fn should_notify(jid: &str, conversation: Option<&Conversation>) -> bool {
    if super::is_group_jid(jid) {
        return conversation.is_some_and(|c| c.notify);
    }
    super::is_private_jid(jid)
}
