use std::time::Duration;

use hub_core::HubContext;
use hub_core::agenda::{self, NewRoutine, NewTask, Priority, quick_add};
use hub_core::chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use hub_core::confirm::CONFIRM_TIMEOUT;
use hub_core::events::HubEvent;
use hub_core::per_os::PerOs;
use hub_core::scripts::{self, NewScript, Origin, runner};
use hub_core::shortcuts::{self, Shortcut, ShortcutKind};
use hub_core::whatsapp;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::json;

const OUTPUT_PREVIEW: usize = 8 * 1024;

#[derive(Clone)]
pub struct HubTools {
    ctx: HubContext,
    confirm_timeout: Duration,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    #[schemars(
        description = "Data inicial (AAAA-MM-DD, DD/MM/AAAA ou texto como hoje, amanhã). Padrão: hoje"
    )]
    pub de: Option<String>,
    #[schemars(description = "Data final inclusiva. Padrão: 7 dias depois da inicial")]
    pub ate: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTaskRequest {
    #[schemars(description = "Título da tarefa ou compromisso")]
    pub titulo: String,
    #[schemars(
        description = "Data (AAAA-MM-DD, DD/MM/AAAA ou texto como amanhã, sexta). Vazio cria sem data"
    )]
    pub data: Option<String>,
    #[schemars(
        description = "Hora no formato HH:MM. Com data e hora o hub cria um lembrete automático"
    )]
    pub hora: Option<String>,
    #[schemars(description = "Notas longas opcionais")]
    pub notas: Option<String>,
    #[schemars(description = "baixa, normal ou alta")]
    pub prioridade: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdRequest {
    #[schemars(description = "Identificador numérico")]
    pub id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateRoutineRequest {
    #[schemars(description = "Título da rotina")]
    pub titulo: String,
    #[schemars(
        description = "Recorrência: diaria, semana:1,3,5 (1 = segunda, 7 = domingo), mes:15 (dia do mês) ou cada:3:AAAA-MM-DD (a cada N dias a partir da data)"
    )]
    pub recorrencia: String,
    #[schemars(description = "Hora HH:MM opcional; cria lembrete para cada dia")]
    pub hora: Option<String>,
    #[schemars(description = "Notas opcionais")]
    pub notas: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateReminderRequest {
    #[schemars(description = "Texto do lembrete. Obrigatório quando não há tarefa_id")]
    pub titulo: Option<String>,
    #[schemars(description = "Tarefa existente a lembrar, opcional")]
    pub tarefa_id: Option<i64>,
    #[schemars(
        description = "Quando disparar: AAAA-MM-DD HH:MM ou texto como em 10 minutos, em 1 hora, amanhã 9h, sexta 18:30"
    )]
    pub quando: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateScriptRequest {
    #[schemars(description = "Nome do script")]
    pub nome: String,
    #[schemars(description = "O que ele faz")]
    pub descricao: Option<String>,
    #[schemars(description = "Nome do arquivo para Windows, por exemplo limpar.ps1")]
    pub arquivo_windows: Option<String>,
    #[schemars(description = "Conteúdo do arquivo para Windows")]
    pub conteudo_windows: Option<String>,
    #[schemars(description = "Nome do arquivo para Linux, por exemplo limpar.sh")]
    pub arquivo_linux: Option<String>,
    #[schemars(description = "Conteúdo do arquivo para Linux")]
    pub conteudo_linux: Option<String>,
    #[schemars(description = "Cron com segundos (0 0 3 * * *) ou 5 campos, opcional")]
    pub agendamento: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScriptIdRequest {
    #[schemars(description = "Id do script (veja scripts_listar)")]
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleRequest {
    #[schemars(description = "Id do script (veja scripts_listar)")]
    pub id: String,
    #[schemars(description = "Cron com segundos ou 5 campos. Vazio remove o agendamento")]
    pub agendamento: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddShortcutRequest {
    #[schemars(description = "Nome do atalho")]
    pub nome: String,
    #[schemars(description = "programa, pasta, url ou comando")]
    pub tipo: String,
    #[schemars(description = "Alvo igual nos dois sistemas")]
    pub alvo: Option<String>,
    #[schemars(description = "Alvo no Windows")]
    pub alvo_windows: Option<String>,
    #[schemars(description = "Alvo no Linux")]
    pub alvo_linux: Option<String>,
    #[schemars(description = "Grupo, padrão Geral")]
    pub grupo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlayRequest {
    #[schemars(description = "Link de vídeo ou playlist do YouTube")]
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WhatsAppReadRequest {
    #[schemars(description = "Conversa: apelido, nome do contato ou grupo, ou o JID")]
    pub conversa: String,
    #[schemars(description = "Quantidade de mensagens mais recentes, padrão 20")]
    pub limite: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WhatsAppSendRequest {
    #[schemars(description = "Conversa: apelido, nome do contato ou grupo, ou o JID")]
    pub conversa: String,
    #[schemars(description = "Texto da mensagem")]
    pub texto: String,
}

pub fn parse_date(text: &str, now: NaiveDateTime) -> Option<NaiveDate> {
    let text = text.trim();
    for format in ["%Y-%m-%d", "%d/%m/%Y"] {
        if let Ok(date) = NaiveDate::parse_from_str(text, format) {
            return Some(date);
        }
    }
    quick_add::parse(text, now).date
}

pub fn parse_moment(text: &str, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let text = text.trim();
    for format in [
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(at) = NaiveDateTime::parse_from_str(text, format) {
            return Some(at);
        }
    }
    quick_add::reminder_time(&quick_add::parse(text, now))
}

fn text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

impl HubTools {
    pub fn new(ctx: HubContext) -> Self {
        Self {
            ctx,
            confirm_timeout: CONFIRM_TIMEOUT,
            tool_router: Self::tool_router(),
        }
    }

    pub fn with_confirm_timeout(mut self, timeout: Duration) -> Self {
        self.confirm_timeout = timeout;
        self
    }

    async fn confirm(&self, title: String, detail: String) -> bool {
        self.ctx
            .confirmations
            .confirm(&self.ctx.events, title, detail, self.confirm_timeout)
            .await
    }

    fn agenda_changed(&self) {
        self.ctx.events.publish(HubEvent::AgendaChanged);
    }
}

#[tool_router]
impl HubTools {
    #[tool(
        description = "Lista tarefas, compromissos, rotinas do dia e lembretes avulsos de um período da agenda do usuário"
    )]
    async fn agenda_listar(
        &self,
        Parameters(request): Parameters<ListRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let from = match request.de.as_deref() {
            Some(text) => parse_date(text, now).ok_or("data inicial inválida")?,
            None => now.date(),
        };
        let to = match request.ate.as_deref() {
            Some(text) => parse_date(text, now).ok_or("data final inválida")?,
            None => from + TimeDelta::days(7),
        };
        let db = self.ctx.db();
        let tasks = agenda::tasks_between(db.conn(), from, to, true).map_err(|e| e.to_string())?;
        let reminders =
            agenda::loose_reminders_between(db.conn(), from, to).map_err(|e| e.to_string())?;
        let undated =
            agenda::pending_before(db.conn(), NaiveDate::MIN).map_err(|e| e.to_string())?;
        let items: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "tipo": if t.routine_id.is_some() { "rotina" } else { "compromisso" },
                    "titulo": t.title,
                    "data": t.date.map(|d| d.to_string()),
                    "hora": t.time.map(|h| h.format("%H:%M").to_string()),
                    "prioridade": t.priority.as_str(),
                    "concluida": t.is_completed(),
                    "notas": t.notes,
                })
            })
            .chain(reminders.iter().map(|r| {
                json!({
                    "id": r.id,
                    "tipo": "lembrete",
                    "titulo": r.title,
                    "quando": r.effective_time().format("%Y-%m-%d %H:%M").to_string(),
                    "disparado": r.fired_at.is_some(),
                    "notas": r.notes,
                })
            }))
            .collect();
        let undated: Vec<serde_json::Value> = undated
            .iter()
            .filter(|t| t.date.is_none())
            .map(|t| json!({ "id": t.id, "titulo": t.title }))
            .collect();
        Ok(text(&json!({
            "de": from.to_string(),
            "ate": to.to_string(),
            "itens": items,
            "sem_data": undated,
        })))
    }

    #[tool(
        description = "Cria uma tarefa ou compromisso na agenda, com data, hora, notas e prioridade opcionais"
    )]
    async fn agenda_criar_tarefa(
        &self,
        Parameters(request): Parameters<CreateTaskRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let date = match request.data.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(text) => Some(parse_date(text, now).ok_or("data inválida")?),
            None => None,
        };
        let time = match request.hora.as_deref().filter(|h| !h.trim().is_empty()) {
            Some(text) => Some(quick_add::parse_time(text).ok_or("hora inválida, use HH:MM")?),
            None => None,
        };
        let priority = match request.prioridade.as_deref() {
            Some(text) => {
                Priority::parse(text.trim()).ok_or("prioridade deve ser baixa, normal ou alta")?
            }
            None => Priority::Normal,
        };
        let task = agenda::create_task(
            self.ctx.db().conn(),
            NewTask {
                title: request.titulo,
                notes: request.notas.unwrap_or_default(),
                date,
                time,
                priority,
            },
            now,
        )
        .map_err(|e| e.to_string())?;
        self.agenda_changed();
        Ok(text(&json!({
            "criada": task.id,
            "titulo": task.title,
            "data": task.date.map(|d| d.to_string()),
            "hora": task.time.map(|h| h.format("%H:%M").to_string()),
            "lembrete": task.due_at().is_some(),
        })))
    }

    #[tool(description = "Marca uma tarefa da agenda como concluída")]
    async fn agenda_concluir(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<String, String> {
        agenda::complete_task(self.ctx.db().conn(), request.id, self.ctx.clock.now())
            .map_err(|e| e.to_string())?;
        self.agenda_changed();
        Ok(format!("Tarefa {} concluída", request.id))
    }

    #[tool(description = "Cria uma rotina recorrente que gera a tarefa do dia automaticamente")]
    async fn agenda_criar_rotina(
        &self,
        Parameters(request): Parameters<CreateRoutineRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let recurrence = request
            .recorrencia
            .trim()
            .parse()
            .map_err(|e: agenda::recurrence::InvalidRecurrence| e.to_string())?;
        let time = match request.hora.as_deref().filter(|h| !h.trim().is_empty()) {
            Some(text) => Some(quick_add::parse_time(text).ok_or("hora inválida, use HH:MM")?),
            None => None,
        };
        let routine = {
            let db = self.ctx.db();
            let routine = agenda::create_routine(
                db.conn(),
                NewRoutine {
                    title: request.titulo,
                    notes: request.notas.unwrap_or_default(),
                    recurrence,
                    time,
                },
            )
            .map_err(|e| e.to_string())?;
            agenda::generate_routine_tasks(db.conn(), now.date(), now)
                .map_err(|e| e.to_string())?;
            routine
        };
        self.agenda_changed();
        Ok(text(&json!({
            "criada": routine.id,
            "titulo": routine.title,
            "recorrencia": routine.recurrence.describe(),
        })))
    }

    #[tool(
        description = "Cria um lembrete que vira notificação no momento indicado, para uma tarefa existente ou avulso"
    )]
    async fn lembrete_criar(
        &self,
        Parameters(request): Parameters<CreateReminderRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let at = parse_moment(&request.quando, now)
            .ok_or("não entendi quando lembrar; use AAAA-MM-DD HH:MM ou algo como em 10 minutos")?;
        let reminder = agenda::create_reminder(
            self.ctx.db().conn(),
            request.tarefa_id,
            request.titulo.as_deref(),
            at,
        )
        .map_err(|e| e.to_string())?;
        self.agenda_changed();
        Ok(text(&json!({
            "criado": reminder.id,
            "titulo": reminder.title,
            "dispara_em": at.format("%Y-%m-%d %H:%M").to_string(),
        })))
    }

    #[tool(
        description = "Lista os scripts do hub com descrição, estado, agendamento e próxima execução"
    )]
    async fn scripts_listar(&self) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let list: Vec<serde_json::Value> = self
            .ctx
            .scripts()
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "nome": s.meta.name,
                    "descricao": s.meta.description,
                    "ativo": s.meta.active,
                    "agendamento": s.meta.schedule,
                    "proxima": s.meta.schedule.as_deref()
                        .filter(|_| s.is_scheduled())
                        .and_then(|cron| scripts::next_run(cron, now))
                        .map(|at| at.format("%Y-%m-%d %H:%M").to_string()),
                    "pode_rodar": s.can_run(),
                    "problema": s.problem,
                })
            })
            .collect();
        Ok(text(&list))
    }

    #[tool(
        description = "Cria um script novo com o código informado. Ele entra inativo e o usuário precisa revisar e ativar no hub"
    )]
    async fn scripts_criar(
        &self,
        Parameters(request): Parameters<CreateScriptRequest>,
    ) -> Result<String, String> {
        let id = scripts::create(
            &self.ctx.paths.scripts_dir,
            NewScript {
                name: request.nome,
                description: request.descricao.unwrap_or_default(),
                windows_file: request.arquivo_windows,
                linux_file: request.arquivo_linux,
                windows_content: request.conteudo_windows.unwrap_or_default(),
                linux_content: request.conteudo_linux.unwrap_or_default(),
                confirm: false,
                timeout_secs: scripts::DEFAULT_TIMEOUT,
                schedule: request.agendamento,
                active: false,
            },
        )
        .map_err(|e| e.to_string())?;
        self.ctx.events.publish(HubEvent::ScriptsChanged);
        Ok(format!(
            "Script {id} criado inativo. Peça ao usuário para revisar e ativar na aba Scripts."
        ))
    }

    #[tool(
        description = "Executa um script existente e devolve o resultado. Pede confirmação ao usuário no hub"
    )]
    async fn scripts_executar(
        &self,
        Parameters(request): Parameters<ScriptIdRequest>,
    ) -> Result<String, String> {
        let script = self.ctx.script(&request.id).map_err(|e| e.to_string())?;
        if !script.meta.active {
            return Err(
                "o script está inativo; o usuário precisa ativar na aba Scripts".to_string(),
            );
        }
        if !script.can_run() {
            return Err(script
                .problem
                .unwrap_or_else(|| "o script não pode rodar".to_string()));
        }
        let approved = self
            .confirm(
                format!("Executar o script {}?", script.meta.name),
                "A IA pediu para executar este script agora.".to_string(),
            )
            .await;
        if !approved {
            return Err("o usuário recusou ou não respondeu a tempo".to_string());
        }
        let (status, output) = runner::run_to_end(self.ctx.clone(), script, Origin::Ai).await;
        let preview = scripts::truncate_output(&output);
        let preview = if preview.len() > OUTPUT_PREVIEW {
            let mut start = preview.len() - OUTPUT_PREVIEW;
            while !preview.is_char_boundary(start) {
                start += 1;
            }
            &preview[start..]
        } else {
            preview
        };
        Ok(text(&json!({
            "resultado": status.describe(),
            "codigo": status.exit_code(),
            "saida": preview,
        })))
    }

    #[tool(
        description = "Define ou remove o agendamento de um script. Pede confirmação ao usuário no hub"
    )]
    async fn scripts_agendar(
        &self,
        Parameters(request): Parameters<ScheduleRequest>,
    ) -> Result<String, String> {
        let script = self.ctx.script(&request.id).map_err(|e| e.to_string())?;
        let schedule = request
            .agendamento
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(cron) = &schedule {
            scripts::parse_schedule(cron).map_err(|e| e.to_string())?;
        }
        let detail = match &schedule {
            Some(cron) => format!("Novo agendamento: {cron}"),
            None => "Remover o agendamento".to_string(),
        };
        let approved = self
            .confirm(
                format!("Alterar o agendamento de {}?", script.meta.name),
                detail,
            )
            .await;
        if !approved {
            return Err("o usuário recusou ou não respondeu a tempo".to_string());
        }
        let mut meta = script.meta.clone();
        meta.schedule = schedule.clone();
        scripts::save_meta(&script, &meta).map_err(|e| e.to_string())?;
        self.ctx.events.publish(HubEvent::ScriptsChanged);
        Ok(match schedule {
            Some(cron) => format!("Agendamento de {} definido para {cron}", script.id),
            None => format!("Agendamento de {} removido", script.id),
        })
    }

    #[tool(description = "Lista os atalhos do hub com tipo, alvo e grupo")]
    async fn atalhos_listar(&self) -> Result<String, String> {
        let loaded = self.ctx.shortcuts();
        let list: Vec<serde_json::Value> = loaded
            .shortcuts
            .iter()
            .enumerate()
            .map(|(index, s)| {
                json!({
                    "indice": index,
                    "nome": s.name,
                    "tipo": format!("{:?}", s.kind).to_lowercase(),
                    "alvo_windows": s.target.windows(),
                    "alvo_linux": s.target.linux(),
                    "grupo": s.group,
                })
            })
            .collect();
        Ok(text(
            &json!({ "atalhos": list, "problema": loaded.problem }),
        ))
    }

    #[tool(description = "Adiciona um atalho para abrir um programa, pasta, link ou comando")]
    async fn atalhos_adicionar(
        &self,
        Parameters(request): Parameters<AddShortcutRequest>,
    ) -> Result<String, String> {
        let kind = ShortcutKind::parse(request.tipo.trim())
            .ok_or("tipo deve ser programa, pasta, url ou comando")?;
        let target = match request.alvo.filter(|a| !a.trim().is_empty()) {
            Some(same) => PerOs::Same(same.trim().to_string()),
            None => PerOs::split(request.alvo_windows, request.alvo_linux),
        };
        let shortcut = Shortcut {
            name: request.nome,
            kind,
            target,
            group: request
                .grupo
                .unwrap_or_else(|| shortcuts::DEFAULT_GROUP.to_string()),
        };
        let index = self
            .ctx
            .shortcut_store()
            .add(&self.ctx.paths.shortcuts_file(), shortcut)
            .map_err(|e| e.to_string())?;
        self.ctx.events.publish(HubEvent::ShortcutsChanged);
        Ok(format!("Atalho criado na posição {index}"))
    }

    #[tool(
        description = "Lê as mensagens mais recentes de uma conversa do WhatsApp guardadas no hub"
    )]
    async fn whatsapp_ler(
        &self,
        Parameters(request): Parameters<WhatsAppReadRequest>,
    ) -> Result<String, String> {
        let db = self.ctx.db();
        let conversation = whatsapp::find_conversation(db.conn(), &request.conversa)
            .map_err(|e| e.to_string())?
            .ok_or("conversa não encontrada no hub")?;
        let limit = request.limite.unwrap_or(20).clamp(1, 100);
        let messages =
            whatsapp::messages(db.conn(), &conversation.jid, limit).map_err(|e| e.to_string())?;
        let items: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                json!({
                    "de": if m.mine { "eu".to_string() } else if m.sender_name.is_empty() { m.sender.clone() } else { m.sender_name.clone() },
                    "texto": m.display_text(),
                    "em": m.at.format("%Y-%m-%d %H:%M").to_string(),
                })
            })
            .collect();
        Ok(text(&json!({
            "conversa": conversation.title(),
            "jid": conversation.jid,
            "grupo": conversation.group,
            "mensagens": items,
        })))
    }

    #[tool(
        description = "Envia uma mensagem de texto no WhatsApp. Pede confirmação ao usuário e exige a aba WhatsApp aberta"
    )]
    async fn whatsapp_enviar(
        &self,
        Parameters(request): Parameters<WhatsAppSendRequest>,
    ) -> Result<String, String> {
        let texto = request.texto.trim().to_string();
        if texto.is_empty() {
            return Err("a mensagem está vazia".to_string());
        }
        let conversation = whatsapp::find_conversation(self.ctx.db().conn(), &request.conversa)
            .map_err(|e| e.to_string())?
            .ok_or("conversa não encontrada no hub")?;
        let handle = self.ctx.whatsapp.get().ok_or(
            "o WhatsApp não está conectado; peça ao usuário para abrir a aba WhatsApp no hub",
        )?;
        let approved = self
            .confirm(
                format!("Enviar mensagem para {}?", conversation.title()),
                texto.clone(),
            )
            .await;
        if !approved {
            return Err("o usuário recusou ou não respondeu a tempo".to_string());
        }
        let sent = handle.send_text(&conversation.jid, &texto).await?;
        let message = whatsapp::wire_to_message(&sent);
        whatsapp::store_message(self.ctx.db().conn(), &message).map_err(|e| e.to_string())?;
        self.ctx.events.publish(HubEvent::WhatsAppChanged);
        Ok(format!("Mensagem enviada para {}", conversation.title()))
    }

    #[tool(
        description = "Toca um link do YouTube (vídeo ou playlist) só em áudio no player do hub"
    )]
    async fn player_tocar(
        &self,
        Parameters(request): Parameters<PlayRequest>,
    ) -> Result<String, String> {
        let url = request.url.trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("informe um link http ou https".to_string());
        }
        self.ctx.events.publish(HubEvent::PlayRequested(url));
        Ok("Pedido enviado ao player do hub".to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HubTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Ferramentas do hub pessoal do usuário: agenda (tarefas, rotinas, lembretes), scripts, atalhos, player e WhatsApp. Datas no fuso local.",
        )
    }
}
