use std::time::Duration;

use hub_core::HubContext;
use hub_core::agenda::{self, NewRoutine, NewTask, Priority, Reminder, Routine, quick_add};
use hub_core::chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use hub_core::confirm::CONFIRM_TIMEOUT;
use hub_core::events::HubEvent;
use hub_core::notes;
use hub_core::per_os::PerOs;
use hub_core::platform::Os;
use hub_core::platform::terminal;
use hub_core::player::{self, PlayerCommand};
use hub_core::scripts::{self, NewScript, Origin, runner};
use hub_core::shortcuts::{self, MoveTarget, Shortcut, ShortcutKind};
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NotesListRequest {
    #[schemars(description = "Texto a procurar no título ou no corpo. Vazio lista todas")]
    pub busca: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoteCreateRequest {
    #[schemars(description = "Título da nota")]
    pub titulo: Option<String>,
    #[schemars(description = "Texto da nota")]
    pub corpo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoteEditRequest {
    #[schemars(description = "Id da nota (veja notas_listar)")]
    pub id: i64,
    #[schemars(description = "Novo título")]
    pub titulo: Option<String>,
    #[schemars(description = "Novo texto; texto vazio limpa o corpo")]
    pub corpo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoteFavoriteRequest {
    #[schemars(description = "Id da nota (veja notas_listar)")]
    pub id: i64,
    #[schemars(description = "true marca como favorita (padrão), false desmarca")]
    pub favorita: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditTaskRequest {
    #[schemars(description = "Id da tarefa ou compromisso (veja agenda_listar)")]
    pub id: i64,
    #[schemars(description = "Novo título")]
    pub titulo: Option<String>,
    #[schemars(description = "Novas notas; texto vazio apaga as notas")]
    pub notas: Option<String>,
    #[schemars(
        description = "Nova data (AAAA-MM-DD, DD/MM/AAAA ou texto como amanhã); texto vazio deixa sem data"
    )]
    pub data: Option<String>,
    #[schemars(description = "Nova hora HH:MM; texto vazio remove a hora")]
    pub hora: Option<String>,
    #[schemars(description = "baixa, normal ou alta")]
    pub prioridade: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ItemRequest {
    #[schemars(description = "Id do item (veja agenda_listar ou lembretes_listar)")]
    pub id: i64,
    #[schemars(
        description = "tarefa (padrão, vale para compromissos e tarefas de rotina) ou lembrete"
    )]
    pub tipo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveRequest {
    #[schemars(description = "Id do item (veja agenda_listar)")]
    pub id: i64,
    #[schemars(description = "tarefa (padrão) ou lembrete")]
    pub tipo: Option<String>,
    #[schemars(
        description = "Nova data (AAAA-MM-DD, DD/MM/AAAA ou texto como amanhã, sexta). A hora é mantida. Para tarefas, texto vazio deixa sem data"
    )]
    pub data: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditRoutineRequest {
    #[schemars(description = "Id da rotina (veja rotinas_listar)")]
    pub id: i64,
    #[schemars(description = "Novo título")]
    pub titulo: Option<String>,
    #[schemars(
        description = "Nova recorrência: diaria, semana:1,3,5 (1 = segunda, 7 = domingo), mes:15 ou cada:3:AAAA-MM-DD"
    )]
    pub recorrencia: Option<String>,
    #[schemars(description = "Nova hora HH:MM; texto vazio remove a hora")]
    pub hora: Option<String>,
    #[schemars(description = "Novas notas; texto vazio apaga")]
    pub notas: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoutineActiveRequest {
    #[schemars(description = "Id da rotina (veja rotinas_listar)")]
    pub id: i64,
    #[schemars(description = "true ativa, false pausa a rotina")]
    pub ativa: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditReminderRequest {
    #[schemars(description = "Id do lembrete (veja lembretes_listar)")]
    pub id: i64,
    #[schemars(description = "Novo texto do lembrete")]
    pub titulo: Option<String>,
    #[schemars(description = "Novas notas; texto vazio apaga")]
    pub notas: Option<String>,
    #[schemars(
        description = "Novo momento: AAAA-MM-DD HH:MM ou texto como em 10 minutos, amanhã 9h"
    )]
    pub quando: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditScriptRequest {
    #[schemars(description = "Id do script (veja scripts_listar)")]
    pub id: String,
    #[schemars(description = "Novo nome exibido (o id não muda)")]
    pub nome: Option<String>,
    #[schemars(description = "Nova descrição")]
    pub descricao: Option<String>,
    #[schemars(description = "Se o hub deve pedir confirmação antes de rodar manualmente")]
    pub confirmar_antes: Option<bool>,
    #[schemars(description = "Tempo limite em segundos")]
    pub timeout_segundos: Option<u64>,
    #[schemars(description = "Cron com segundos ou 5 campos; texto vazio remove o agendamento")]
    pub agendamento: Option<String>,
    #[schemars(description = "Ativa ou desativa o script")]
    pub ativo: Option<bool>,
    #[schemars(
        description = "Nome do arquivo para Windows (sem pastas); texto vazio remove a versão Windows"
    )]
    pub arquivo_windows: Option<String>,
    #[schemars(
        description = "Nome do arquivo para Linux (sem pastas); texto vazio remove a versão Linux"
    )]
    pub arquivo_linux: Option<String>,
    #[schemars(description = "Novo código completo do arquivo Windows")]
    pub conteudo_windows: Option<String>,
    #[schemars(description = "Novo código completo do arquivo Linux")]
    pub conteudo_linux: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryRequest {
    #[schemars(description = "Id do script; sem id devolve a última execução de cada script")]
    pub id: Option<String>,
    #[schemars(description = "Quantidade de execuções, padrão 10, máximo 50")]
    pub limite: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShortcutIndexRequest {
    #[schemars(description = "Índice do atalho (veja atalhos_listar)")]
    pub indice: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveShortcutRequest {
    #[schemars(description = "Índice do atalho a mover (veja atalhos_listar)")]
    pub indice: usize,
    #[schemars(
        description = "Índice do atalho que vai ocupar a posição; o atalho movido entra no grupo dele. No mesmo grupo, vindo de antes, fica logo depois dele"
    )]
    pub para_indice: Option<usize>,
    #[schemars(
        description = "Grupo de destino, usado quando não há para_indice; o atalho vai para o fim do grupo"
    )]
    pub grupo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveGroupRequest {
    #[schemars(description = "Nome do grupo a mover")]
    pub grupo: String,
    #[schemars(
        description = "Grupo que vai ocupar a posição: vindo de antes, fica depois dele; vindo de depois, fica antes dele"
    )]
    pub para_grupo: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CollapseGroupRequest {
    #[schemars(description = "Nome do grupo")]
    pub grupo: String,
    #[schemars(description = "true recolhe, false expande")]
    pub recolhido: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenShortcutRequest {
    #[schemars(description = "Índice do atalho (veja atalhos_listar)")]
    pub indice: Option<usize>,
    #[schemars(description = "Nome do atalho, usado quando não há índice")]
    pub nome: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditShortcutRequest {
    #[schemars(description = "Índice do atalho (veja atalhos_listar)")]
    pub indice: usize,
    #[schemars(description = "Novo nome")]
    pub nome: Option<String>,
    #[schemars(description = "programa, pasta, url ou comando")]
    pub tipo: Option<String>,
    #[schemars(description = "Novo alvo igual nos dois sistemas")]
    pub alvo: Option<String>,
    #[schemars(description = "Novo alvo no Windows")]
    pub alvo_windows: Option<String>,
    #[schemars(description = "Novo alvo no Linux")]
    pub alvo_linux: Option<String>,
    #[schemars(description = "Novo grupo")]
    pub grupo: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VolumeRequest {
    #[schemars(description = "Volume de 0 a 100")]
    pub volume: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FavoriteRequest {
    #[schemars(description = "Link do YouTube (vídeo ou playlist)")]
    pub url: String,
    #[schemars(description = "Nome para exibir, opcional")]
    pub titulo: Option<String>,
    #[schemars(description = "true remove dos favoritos")]
    pub remover: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConversationsRequest {
    #[schemars(description = "Filtro por nome, apelido ou JID, opcional")]
    pub busca: Option<String>,
    #[schemars(description = "Quantidade máxima, padrão 30")]
    pub limite: Option<usize>,
    #[schemars(
        description = "true lista só as arquivadas, false só as não arquivadas; omitido lista todas"
    )]
    pub arquivadas: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AliasRequest {
    #[schemars(description = "Conversa: apelido atual, nome do contato ou grupo, ou o JID")]
    pub conversa: String,
    #[schemars(description = "Novo apelido; texto vazio remove o apelido")]
    pub apelido: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TerminalRequest {
    #[schemars(
        description = "Comando a executar: PowerShell no Windows, sh no Linux. O usuário confirma cada comando"
    )]
    pub comando: String,
    #[schemars(
        description = "Pasta de trabalho; aceita ~ para a pasta pessoal. Padrão: pasta pessoal"
    )]
    pub pasta: Option<String>,
    #[schemars(description = "Tempo limite em segundos, padrão 60, máximo 600")]
    pub tempo_limite_segundos: Option<u64>,
}

const REFUSED: &str = "o usuário recusou ou não respondeu a tempo";
const HISTORY_OUTPUT: usize = 2000;

fn err<E: ToString>(error: E) -> String {
    error.to_string()
}

fn filled(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn hour(time: Option<NaiveTime>) -> Option<String> {
    time.map(|h| h.format("%H:%M").to_string())
}

fn parse_hour(text: &str) -> Result<Option<NaiveTime>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    quick_add::parse_time(text)
        .map(Some)
        .ok_or_else(|| "hora inválida, use HH:MM".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Task,
    Reminder,
}

fn item_kind(text: Option<&str>) -> Result<ItemKind, String> {
    match text.map(|t| t.trim().to_lowercase()).as_deref() {
        None | Some("" | "tarefa" | "compromisso" | "rotina") => Ok(ItemKind::Task),
        Some("lembrete") => Ok(ItemKind::Reminder),
        Some(other) => Err(format!("tipo {other} desconhecido; use tarefa ou lembrete")),
    }
}

fn kind_name(kind: ShortcutKind) -> &'static str {
    match kind {
        ShortcutKind::Program => "programa",
        ShortcutKind::Folder => "pasta",
        ShortcutKind::Url => "url",
        ShortcutKind::Command => "comando",
    }
}

fn note_json(note: &notes::Note) -> serde_json::Value {
    json!({
        "id": note.id,
        "titulo": note.display_title(),
        "corpo": note.body,
        "favorita": note.favorite,
        "temporaria": note.reset.label(),
        "criada_em": note.created_at.format("%Y-%m-%d %H:%M").to_string(),
        "atualizada_em": note.updated_at.format("%Y-%m-%d %H:%M").to_string(),
    })
}

fn reminder_json(reminder: &Reminder) -> serde_json::Value {
    json!({
        "id": reminder.id,
        "titulo": reminder.title,
        "quando": reminder.effective_time().format("%Y-%m-%d %H:%M").to_string(),
        "adiado": reminder.snoozed_until.is_some(),
        "disparado": reminder.fired_at.is_some(),
        "tarefa_id": reminder.task_id,
        "notas": reminder.notes,
    })
}

fn routine_json(routine: &Routine) -> serde_json::Value {
    json!({
        "id": routine.id,
        "titulo": routine.title,
        "recorrencia": routine.recurrence.to_string(),
        "descricao": routine.recurrence.describe(),
        "hora": hour(routine.time),
        "ativa": routine.active,
        "notas": routine.notes,
    })
}

fn os_label(os: Os) -> &'static str {
    match os {
        Os::Windows => "Windows",
        Os::Linux => "Linux",
    }
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

    fn notes_changed(&self) {
        self.ctx.events.publish(HubEvent::NotesChanged);
    }

    fn agenda_changed(&self) {
        self.ctx.events.publish(HubEvent::AgendaChanged);
    }

    fn player(&self, command: PlayerCommand) -> String {
        self.ctx.events.publish(HubEvent::PlayerControl(command));
        "Pedido enviado ao player do hub".to_string()
    }

    fn find_shortcut(
        &self,
        index: Option<usize>,
        name: Option<&str>,
    ) -> Result<(usize, Shortcut), String> {
        let loaded = self.ctx.shortcuts();
        if let Some(index) = index {
            return loaded
                .shortcuts
                .get(index)
                .cloned()
                .map(|s| (index, s))
                .ok_or_else(|| format!("atalho {index} não encontrado"));
        }
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .ok_or("informe o índice ou o nome do atalho")?;
        let lower = name.to_lowercase();
        let list = &loaded.shortcuts;
        list.iter()
            .position(|s| s.name.to_lowercase() == lower)
            .or_else(|| list.iter().position(|s| shortcuts::matches(s, name)))
            .map(|index| (index, list[index].clone()))
            .ok_or_else(|| format!("nenhum atalho chamado {name}"))
    }
}

#[tool_router]
impl HubTools {
    #[tool(
        description = "Lista tarefas, compromissos, tarefas de rotina, lembretes avulsos e previsões de rotina (dias futuros) de um período da agenda do usuário"
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
        let projections =
            agenda::routine_projections(db.conn(), from, to, now.date()).map_err(err)?;
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
            .chain(projections.iter().map(|p| {
                json!({
                    "tipo": "previsao_rotina",
                    "previsao": true,
                    "rotina_id": p.routine.id,
                    "titulo": p.routine.title,
                    "data": p.date.to_string(),
                    "hora": hour(p.routine.time),
                    "notas": p.routine.notes,
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
            "nota": "Itens com tipo previsao_rotina são previsões de rotina: a tarefa ainda não existe e será criada automaticamente no dia. Para alterar, edite a rotina pelo rotina_id.",
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
                    "tipo": kind_name(s.kind),
                    "alvo_windows": s.target.windows(),
                    "alvo_linux": s.target.linux(),
                    "grupo": s.group,
                })
            })
            .collect();
        Ok(text(&json!({
            "atalhos": list,
            "grupos": shortcuts::groups(&loaded.shortcuts),
            "recolhidos": loaded.collapsed,
            "problema": loaded.problem,
        })))
    }

    #[tool(
        description = "Muda a posição de um atalho: para a posição de outro atalho (para_indice) ou para o fim de um grupo (grupo). Os índices mudam depois de mover"
    )]
    async fn atalhos_mover(
        &self,
        Parameters(request): Parameters<MoveShortcutRequest>,
    ) -> Result<String, String> {
        let target = match (request.para_indice, request.grupo) {
            (Some(index), _) => MoveTarget::Shortcut(index),
            (None, Some(group)) if !group.trim().is_empty() => {
                MoveTarget::GroupEnd(group.trim().to_string())
            }
            _ => return Err("informe para_indice ou grupo".to_string()),
        };
        if let MoveTarget::Shortcut(index) = target
            && index >= self.ctx.shortcuts().shortcuts.len()
        {
            return Err(format!("atalho {index} não encontrado"));
        }
        self.ctx
            .shortcut_store()
            .move_shortcut(&self.ctx.paths.shortcuts_file(), request.indice, target)
            .map_err(err)?;
        self.ctx.events.publish(HubEvent::ShortcutsChanged);
        Ok("Atalho movido".to_string())
    }

    #[tool(description = "Muda a ordem dos grupos de atalhos")]
    async fn atalhos_grupo_mover(
        &self,
        Parameters(request): Parameters<MoveGroupRequest>,
    ) -> Result<String, String> {
        let groups = shortcuts::groups(&self.ctx.shortcuts().shortcuts);
        for group in [&request.grupo, &request.para_grupo] {
            if !groups.contains(group) {
                return Err(format!("grupo {group} não encontrado"));
            }
        }
        self.ctx
            .shortcut_store()
            .move_group(
                &self.ctx.paths.shortcuts_file(),
                &request.grupo,
                &request.para_grupo,
            )
            .map_err(err)?;
        self.ctx.events.publish(HubEvent::ShortcutsChanged);
        Ok(format!("Grupo {} movido", request.grupo))
    }

    #[tool(description = "Recolhe ou expande um grupo de atalhos na aba Atalhos")]
    async fn atalhos_grupo_recolher(
        &self,
        Parameters(request): Parameters<CollapseGroupRequest>,
    ) -> Result<String, String> {
        if !shortcuts::groups(&self.ctx.shortcuts().shortcuts).contains(&request.grupo) {
            return Err(format!("grupo {} não encontrado", request.grupo));
        }
        self.ctx
            .shortcut_store()
            .set_collapsed(
                &self.ctx.paths.shortcuts_file(),
                &request.grupo,
                request.recolhido,
            )
            .map_err(err)?;
        self.ctx.events.publish(HubEvent::ShortcutsChanged);
        Ok(format!(
            "Grupo {} {}",
            request.grupo,
            if request.recolhido {
                "recolhido"
            } else {
                "expandido"
            }
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
                    "de": if m.mine { "eu".to_string() } else { m.sender_label() },
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

    #[tool(
        description = "Edita uma tarefa ou compromisso: título, notas, data, hora e prioridade. Campos omitidos ficam como estão"
    )]
    async fn agenda_editar_tarefa(
        &self,
        Parameters(request): Parameters<EditTaskRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let task = {
            let db = self.ctx.db();
            let current = agenda::get_task(db.conn(), request.id).map_err(err)?;
            let date = match request.data.as_deref() {
                Some(text) if text.trim().is_empty() => None,
                Some(text) => Some(parse_date(text, now).ok_or("data inválida")?),
                None => current.date,
            };
            let time = match request.hora.as_deref() {
                Some(text) => parse_hour(text)?,
                None => current.time,
            };
            let priority = match request.prioridade.as_deref() {
                Some(text) => Priority::parse(text.trim())
                    .ok_or("prioridade deve ser baixa, normal ou alta")?,
                None => current.priority,
            };
            agenda::update_task(
                db.conn(),
                request.id,
                NewTask {
                    title: request.titulo.unwrap_or(current.title),
                    notes: request.notas.unwrap_or(current.notes),
                    date,
                    time,
                    priority,
                },
            )
            .map_err(err)?
        };
        self.agenda_changed();
        Ok(text(&json!({
            "editada": task.id,
            "titulo": task.title,
            "data": task.date.map(|d| d.to_string()),
            "hora": hour(task.time),
            "prioridade": task.priority.as_str(),
            "notas": task.notes,
        })))
    }

    #[tool(
        description = "Move uma tarefa ou lembrete para outra data mantendo a hora (como arrastar no quadro da agenda)"
    )]
    async fn agenda_mover(
        &self,
        Parameters(request): Parameters<MoveRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let kind = item_kind(request.tipo.as_deref())?;
        let date = if request.data.trim().is_empty() {
            None
        } else {
            Some(parse_date(&request.data, now).ok_or("data inválida")?)
        };
        let result = {
            let db = self.ctx.db();
            match (kind, date) {
                (ItemKind::Task, date) => {
                    let task = agenda::move_task(db.conn(), request.id, date).map_err(err)?;
                    json!({
                        "movida": task.id,
                        "titulo": task.title,
                        "data": task.date.map(|d| d.to_string()),
                        "hora": hour(task.time),
                    })
                }
                (ItemKind::Reminder, Some(date)) => {
                    let reminder =
                        agenda::move_reminder(db.conn(), request.id, date, now).map_err(err)?;
                    json!({
                        "movido": reminder.id,
                        "titulo": reminder.title,
                        "quando": reminder.effective_time().format("%Y-%m-%d %H:%M").to_string(),
                    })
                }
                (ItemKind::Reminder, None) => {
                    return Err("lembretes precisam de uma data".to_string());
                }
            }
        };
        self.agenda_changed();
        Ok(text(&result))
    }

    #[tool(description = "Exclui uma tarefa, compromisso ou lembrete da agenda")]
    async fn agenda_excluir(
        &self,
        Parameters(request): Parameters<ItemRequest>,
    ) -> Result<String, String> {
        let kind = item_kind(request.tipo.as_deref())?;
        let title = {
            let db = self.ctx.db();
            match kind {
                ItemKind::Task => {
                    let task = agenda::get_task(db.conn(), request.id).map_err(err)?;
                    agenda::delete_task(db.conn(), request.id).map_err(err)?;
                    task.title
                }
                ItemKind::Reminder => {
                    let reminder = agenda::get_reminder(db.conn(), request.id).map_err(err)?;
                    agenda::delete_reminder(db.conn(), request.id).map_err(err)?;
                    reminder.title
                }
            }
        };
        self.agenda_changed();
        Ok(format!("{title} excluído da agenda"))
    }

    #[tool(
        description = "Marca um compromisso atrasado como perdido: sai das pendências e fica marcado no calendário"
    )]
    async fn agenda_marcar_perdido(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<String, String> {
        agenda::set_task_missed(
            self.ctx.db().conn(),
            request.id,
            Some(self.ctx.clock.now()),
        )
        .map_err(err)?;
        self.agenda_changed();
        Ok(format!("Tarefa {} marcada como perdida", request.id))
    }

    #[tool(description = "Reabre uma tarefa concluída ou perdida")]
    async fn agenda_reabrir(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<String, String> {
        agenda::uncomplete_task(self.ctx.db().conn(), request.id).map_err(err)?;
        self.agenda_changed();
        Ok(format!("Tarefa {} reaberta", request.id))
    }

    #[tool(
        description = "Lista as notas do usuário (favoritas primeiro), com busca opcional no título e no corpo"
    )]
    async fn notas_listar(
        &self,
        Parameters(request): Parameters<NotesListRequest>,
    ) -> Result<String, String> {
        let db = self.ctx.db();
        let found = match filled(request.busca) {
            Some(query) => notes::search(db.conn(), &query),
            None => notes::list(db.conn()),
        }
        .map_err(err)?;
        let list: Vec<serde_json::Value> = found.iter().map(note_json).collect();
        Ok(text(&list))
    }

    #[tool(description = "Cria uma nota com título e texto opcionais")]
    async fn notas_criar(
        &self,
        Parameters(request): Parameters<NoteCreateRequest>,
    ) -> Result<String, String> {
        let title = request.titulo.unwrap_or_default();
        let body = request.corpo.unwrap_or_default();
        if title.trim().is_empty() && body.trim().is_empty() {
            return Err("informe o título ou o corpo da nota".to_string());
        }
        let now = self.ctx.clock.now();
        let note = notes::create(self.ctx.db().conn(), title.trim(), &body, now).map_err(err)?;
        self.notes_changed();
        Ok(text(
            &json!({ "criada": note.id, "titulo": note.display_title() }),
        ))
    }

    #[tool(description = "Edita o título e o texto de uma nota; só muda o que for informado")]
    async fn notas_editar(
        &self,
        Parameters(request): Parameters<NoteEditRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let title = {
            let db = self.ctx.db();
            let note = notes::get(db.conn(), request.id)
                .map_err(err)?
                .ok_or_else(|| format!("nota {} não encontrada", request.id))?;
            let title = request
                .titulo
                .map(|t| t.trim().to_string())
                .unwrap_or(note.title);
            let body = request.corpo.unwrap_or(note.body);
            notes::update(db.conn(), request.id, &title, &body, now).map_err(err)?;
            title
        };
        self.notes_changed();
        Ok(format!("Nota {} atualizada ({title})", request.id))
    }

    #[tool(description = "Marca ou desmarca uma nota como favorita")]
    async fn notas_favoritar(
        &self,
        Parameters(request): Parameters<NoteFavoriteRequest>,
    ) -> Result<String, String> {
        let favorite = request.favorita.unwrap_or(true);
        {
            let db = self.ctx.db();
            notes::get(db.conn(), request.id)
                .map_err(err)?
                .ok_or_else(|| format!("nota {} não encontrada", request.id))?;
            notes::set_favorite(db.conn(), request.id, favorite).map_err(err)?;
        }
        self.notes_changed();
        Ok(if favorite {
            format!("Nota {} marcada como favorita", request.id)
        } else {
            format!("Nota {} desmarcada como favorita", request.id)
        })
    }

    #[tool(description = "Apaga uma nota. Pede confirmação ao usuário no hub")]
    async fn notas_apagar(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<String, String> {
        let note = notes::get(self.ctx.db().conn(), request.id)
            .map_err(err)?
            .ok_or_else(|| format!("nota {} não encontrada", request.id))?;
        let approved = self
            .confirm(
                format!("Apagar a nota {}?", note.display_title()),
                "A IA pediu para apagar esta nota. A ação não pode ser desfeita".to_string(),
            )
            .await;
        if !approved {
            return Err(REFUSED.to_string());
        }
        notes::delete(self.ctx.db().conn(), request.id).map_err(err)?;
        self.notes_changed();
        Ok(format!("Nota {} apagada", request.id))
    }

    #[tool(description = "Lista as rotinas com recorrência, hora, notas e se estão ativas")]
    async fn rotinas_listar(&self) -> Result<String, String> {
        let routines = agenda::list_routines(self.ctx.db().conn()).map_err(err)?;
        let list: Vec<serde_json::Value> = routines.iter().map(routine_json).collect();
        Ok(text(&list))
    }

    #[tool(
        description = "Edita uma rotina: título, recorrência, hora e notas. Vale para os próximos dias; tarefas já geradas não mudam"
    )]
    async fn rotinas_editar(
        &self,
        Parameters(request): Parameters<EditRoutineRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let routine = {
            let db = self.ctx.db();
            let current = agenda::get_routine(db.conn(), request.id).map_err(err)?;
            let recurrence = match request.recorrencia.as_deref() {
                Some(text) => text
                    .trim()
                    .parse()
                    .map_err(|e: agenda::recurrence::InvalidRecurrence| e.to_string())?,
                None => current.recurrence,
            };
            let time = match request.hora.as_deref() {
                Some(text) => parse_hour(text)?,
                None => current.time,
            };
            let routine = agenda::update_routine(
                db.conn(),
                request.id,
                NewRoutine {
                    title: request.titulo.unwrap_or(current.title),
                    notes: request.notas.unwrap_or(current.notes),
                    recurrence,
                    time,
                },
            )
            .map_err(err)?;
            if routine.active {
                agenda::generate_routine_tasks(db.conn(), now.date(), now).map_err(err)?;
            }
            routine
        };
        self.agenda_changed();
        Ok(text(&routine_json(&routine)))
    }

    #[tool(description = "Ativa ou pausa uma rotina")]
    async fn rotinas_ativar(
        &self,
        Parameters(request): Parameters<RoutineActiveRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        {
            let db = self.ctx.db();
            agenda::set_routine_active(db.conn(), request.id, request.ativa).map_err(err)?;
            if request.ativa {
                agenda::generate_routine_tasks(db.conn(), now.date(), now).map_err(err)?;
            }
        }
        self.agenda_changed();
        Ok(if request.ativa {
            format!("Rotina {} ativada", request.id)
        } else {
            format!("Rotina {} pausada", request.id)
        })
    }

    #[tool(description = "Exclui uma rotina")]
    async fn rotinas_excluir(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<String, String> {
        let title = {
            let db = self.ctx.db();
            let routine = agenda::get_routine(db.conn(), request.id).map_err(err)?;
            agenda::delete_routine(db.conn(), request.id).map_err(err)?;
            routine.title
        };
        self.agenda_changed();
        Ok(format!("Rotina {title} excluída"))
    }

    #[tool(
        description = "Lista os lembretes pendentes (ainda não disparados), avulsos ou ligados a tarefas"
    )]
    async fn lembretes_listar(&self) -> Result<String, String> {
        let reminders = agenda::pending_reminders(self.ctx.db().conn()).map_err(err)?;
        let list: Vec<serde_json::Value> = reminders.iter().map(reminder_json).collect();
        Ok(text(&list))
    }

    #[tool(
        description = "Edita um lembrete: texto, notas e momento. Campos omitidos ficam como estão"
    )]
    async fn lembrete_editar(
        &self,
        Parameters(request): Parameters<EditReminderRequest>,
    ) -> Result<String, String> {
        let now = self.ctx.clock.now();
        let reminder = {
            let db = self.ctx.db();
            let current = agenda::get_reminder(db.conn(), request.id).map_err(err)?;
            let at = match request.quando.as_deref() {
                Some(text) => parse_moment(text, now).ok_or(
                    "não entendi quando lembrar; use AAAA-MM-DD HH:MM ou algo como em 10 minutos",
                )?,
                None => current.effective_time(),
            };
            agenda::update_reminder(
                db.conn(),
                request.id,
                request.titulo.as_deref().unwrap_or(&current.title),
                request.notas.as_deref().unwrap_or(&current.notes),
                at,
                now,
            )
            .map_err(err)?
        };
        self.agenda_changed();
        Ok(text(&reminder_json(&reminder)))
    }

    #[tool(description = "Exclui um lembrete")]
    async fn lembrete_excluir(
        &self,
        Parameters(request): Parameters<IdRequest>,
    ) -> Result<String, String> {
        agenda::delete_reminder(self.ctx.db().conn(), request.id).map_err(err)?;
        self.agenda_changed();
        Ok(format!("Lembrete {} excluído", request.id))
    }

    #[tool(
        description = "Lê um script: metadados (nome, descrição, arquivos, agendamento, estado) e o código de cada sistema"
    )]
    async fn scripts_ler(
        &self,
        Parameters(request): Parameters<ScriptIdRequest>,
    ) -> Result<String, String> {
        let script = self.ctx.script(&request.id).map_err(err)?;
        let dir = &self.ctx.paths.scripts_dir;
        let code = |os| scripts::read_code(dir, &script.meta, os).unwrap_or(None);
        Ok(text(&json!({
            "id": script.id,
            "nome": script.meta.name,
            "descricao": script.meta.description,
            "ativo": script.meta.active,
            "confirmar_antes": script.meta.confirm,
            "timeout_segundos": script.meta.timeout_secs,
            "agendamento": script.meta.schedule,
            "arquivo_windows": script.meta.file.windows(),
            "arquivo_linux": script.meta.file.linux(),
            "codigo_windows": code(Os::Windows),
            "codigo_linux": code(Os::Linux),
            "pode_rodar": script.can_run(),
            "problema": script.problem,
        })))
    }

    #[tool(
        description = "Edita um script: metadados, agendamento, arquivos e código. Mudanças de código, agendamento, arquivos, ativação ou confirmação pedem confirmação ao usuário no hub, mostrando o código novo"
    )]
    async fn scripts_editar(
        &self,
        Parameters(request): Parameters<EditScriptRequest>,
    ) -> Result<String, String> {
        let script = self.ctx.script(&request.id).map_err(err)?;
        let dir = self.ctx.paths.scripts_dir.clone();
        let old = script.meta.clone();
        let mut meta = old.clone();
        if let Some(name) = request.nome {
            meta.name = name.trim().to_string();
        }
        if let Some(description) = request.descricao {
            meta.description = description.trim().to_string();
        }
        if let Some(confirm) = request.confirmar_antes {
            meta.confirm = confirm;
        }
        if let Some(timeout) = request.timeout_segundos {
            meta.timeout_secs = timeout.max(1);
        }
        if let Some(active) = request.ativo {
            meta.active = active;
        }
        if let Some(schedule) = request.agendamento {
            meta.schedule = filled(Some(schedule));
            if let Some(cron) = &meta.schedule {
                scripts::parse_schedule(cron).map_err(err)?;
            }
        }
        if request.arquivo_windows.is_some() || request.arquivo_linux.is_some() {
            let windows = request
                .arquivo_windows
                .or_else(|| old.file.windows().map(str::to_string));
            let linux = request
                .arquivo_linux
                .or_else(|| old.file.linux().map(str::to_string));
            meta.file = PerOs::split(windows, linux);
            if meta.file.windows().is_none() && meta.file.linux().is_none() {
                return Err(scripts::ScriptError::NoFile.to_string());
            }
            for name in [meta.file.windows(), meta.file.linux()]
                .into_iter()
                .flatten()
            {
                scripts::validate_file_name(name).map_err(err)?;
            }
        }
        if meta.name.is_empty() {
            return Err(scripts::ScriptError::EmptyName.to_string());
        }
        let code: Vec<(Os, String)> = [
            (Os::Windows, request.conteudo_windows),
            (Os::Linux, request.conteudo_linux),
        ]
        .into_iter()
        .filter_map(|(os, content)| content.map(|c| (os, c)))
        .collect();
        for (os, _) in &code {
            scripts::code_path(&dir, &meta, *os)
                .map_err(|e| format!("{} ({}): {e}", os_label(*os), meta.name))?;
        }
        if code.is_empty() && meta == old {
            return Err("nada para alterar".to_string());
        }
        let mut changes: Vec<String> = Vec::new();
        if meta.schedule != old.schedule {
            changes.push(match &meta.schedule {
                Some(cron) => format!("Novo agendamento: {cron}"),
                None => "Remover o agendamento".to_string(),
            });
        }
        if meta.active && !old.active {
            changes.push("Ativar o script".to_string());
        }
        if !meta.confirm && old.confirm {
            changes.push("Deixar de pedir confirmação antes de rodar".to_string());
        }
        if meta.file != old.file {
            changes.push(format!(
                "Arquivos: Windows {}, Linux {}",
                meta.file.windows().unwrap_or("(nenhum)"),
                meta.file.linux().unwrap_or("(nenhum)")
            ));
        }
        for (os, content) in &code {
            changes.push(format!(
                "Novo código para {} ({}):\n{content}",
                os_label(*os),
                meta.file.for_os(*os).unwrap_or_default()
            ));
        }
        let confirmed = !changes.is_empty();
        if confirmed {
            let approved = self
                .confirm(
                    format!("Alterar o script {}?", old.name),
                    changes.join("\n\n"),
                )
                .await;
            if !approved {
                return Err(REFUSED.to_string());
            }
        }
        for (os, content) in &code {
            scripts::write_code(&dir, &meta, *os, content).map_err(err)?;
        }
        if meta != old {
            scripts::save_meta(&script, &meta).map_err(err)?;
        }
        self.ctx.events.publish(HubEvent::ScriptsChanged);
        let written: Vec<&str> = code.iter().map(|(os, _)| os_label(*os)).collect();
        Ok(text(&json!({
            "alterado": script.id,
            "codigo_gravado": written,
            "pediu_confirmacao": confirmed,
        })))
    }

    #[tool(
        description = "Exclui um script e seus arquivos de código. Pede confirmação ao usuário no hub"
    )]
    async fn scripts_excluir(
        &self,
        Parameters(request): Parameters<ScriptIdRequest>,
    ) -> Result<String, String> {
        let script = self.ctx.script(&request.id).map_err(err)?;
        let files: Vec<&str> = [script.meta.file.windows(), script.meta.file.linux()]
            .into_iter()
            .flatten()
            .collect();
        let approved = self
            .confirm(
                format!("Excluir o script {}?", script.meta.name),
                format!(
                    "A IA pediu para excluir este script. Arquivos apagados: {}.toml, {}",
                    script.id,
                    files.join(", ")
                ),
            )
            .await;
        if !approved {
            return Err(REFUSED.to_string());
        }
        scripts::delete(&self.ctx.paths.scripts_dir, &script).map_err(err)?;
        self.ctx.events.publish(HubEvent::ScriptsChanged);
        Ok(format!("Script {} excluído", script.id))
    }

    #[tool(
        description = "Mostra as execuções recentes de um script (início, fim, código de saída, origem e final da saída)"
    )]
    async fn scripts_historico(
        &self,
        Parameters(request): Parameters<HistoryRequest>,
    ) -> Result<String, String> {
        let limit = request.limite.unwrap_or(10).clamp(1, 50);
        let id = filled(request.id);
        if let Some(id) = &id {
            self.ctx.script(id).map_err(err)?;
        }
        let runs = {
            let db = self.ctx.db();
            match &id {
                Some(id) => scripts::executions(db.conn(), id, limit).map_err(err)?,
                None => {
                    let mut all: Vec<scripts::Execution> = scripts::last_executions(db.conn())
                        .map_err(err)?
                        .into_values()
                        .collect();
                    all.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.id.cmp(&a.id)));
                    all.truncate(limit);
                    all
                }
            }
        };
        let list: Vec<serde_json::Value> = runs
            .iter()
            .map(|run| {
                let (output, cut) = terminal::truncate(&run.output, HISTORY_OUTPUT);
                json!({
                    "script": run.script,
                    "inicio": run.started_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                    "fim": run.finished_at.map(|at| at.format("%Y-%m-%d %H:%M:%S").to_string()),
                    "codigo": run.exit_code,
                    "sucesso": run.succeeded(),
                    "origem": run.origin.label(),
                    "saida": output,
                    "saida_cortada": cut,
                })
            })
            .collect();
        Ok(text(&list))
    }

    #[tool(
        description = "Edita um atalho pelo índice (veja atalhos_listar). Campos omitidos ficam como estão"
    )]
    async fn atalhos_editar(
        &self,
        Parameters(request): Parameters<EditShortcutRequest>,
    ) -> Result<String, String> {
        let (index, current) = self.find_shortcut(Some(request.indice), None)?;
        let kind = match request.tipo.as_deref() {
            Some(text) => ShortcutKind::parse(text.trim())
                .ok_or("tipo deve ser programa, pasta, url ou comando")?,
            None => current.kind,
        };
        let target = match filled(request.alvo) {
            Some(same) => PerOs::Same(same),
            None if request.alvo_windows.is_some() || request.alvo_linux.is_some() => PerOs::split(
                request
                    .alvo_windows
                    .or_else(|| current.target.windows().map(str::to_string)),
                request
                    .alvo_linux
                    .or_else(|| current.target.linux().map(str::to_string)),
            ),
            None => current.target.clone(),
        };
        let mut shortcut = current;
        if let Some(name) = request.nome {
            shortcut.name = name;
        }
        if let Some(group) = request.grupo {
            shortcut.group = group;
        }
        shortcut.kind = kind;
        shortcut.target = target;
        self.ctx
            .shortcut_store()
            .update(&self.ctx.paths.shortcuts_file(), index, shortcut)
            .map_err(err)?;
        self.ctx.events.publish(HubEvent::ShortcutsChanged);
        Ok(format!("Atalho {index} atualizado"))
    }

    #[tool(description = "Exclui um atalho pelo índice (veja atalhos_listar)")]
    async fn atalhos_excluir(
        &self,
        Parameters(request): Parameters<ShortcutIndexRequest>,
    ) -> Result<String, String> {
        let removed = self
            .ctx
            .shortcut_store()
            .remove(&self.ctx.paths.shortcuts_file(), request.indice)
            .map_err(err)?;
        self.ctx.events.publish(HubEvent::ShortcutsChanged);
        Ok(format!("Atalho {} excluído", removed.name))
    }

    #[tool(
        description = "Abre um atalho pelo índice ou nome. Atalhos de programa ou comando pedem confirmação ao usuário no hub"
    )]
    async fn atalhos_abrir(
        &self,
        Parameters(request): Parameters<OpenShortcutRequest>,
    ) -> Result<String, String> {
        let (_, shortcut) = self.find_shortcut(request.indice, request.nome.as_deref())?;
        if matches!(shortcut.kind, ShortcutKind::Program | ShortcutKind::Command) {
            let target = shortcut
                .target
                .for_os(self.ctx.platform.os())
                .unwrap_or_default()
                .to_string();
            let approved = self
                .confirm(
                    format!("Abrir o atalho {}?", shortcut.name),
                    format!("{}: {target}", shortcut.kind.label()),
                )
                .await;
            if !approved {
                return Err(REFUSED.to_string());
            }
        }
        shortcuts::open(self.ctx.platform.as_ref(), &shortcut).map_err(err)?;
        let key = shortcuts::usage_key(&shortcut, self.ctx.platform.os());
        let recorded = shortcuts::record_use(self.ctx.db().conn(), &key, self.ctx.clock.now());
        if recorded.is_ok() {
            self.ctx.events.publish(HubEvent::ShortcutsChanged);
        }
        Ok(format!("Atalho {} aberto", shortcut.name))
    }

    #[tool(description = "Pausa o que está tocando no player do hub ou na mídia do sistema")]
    async fn player_pausar(&self) -> Result<String, String> {
        Ok(self.player(PlayerCommand::Pause))
    }

    #[tool(description = "Retoma a reprodução pausada no player do hub ou na mídia do sistema")]
    async fn player_continuar(&self) -> Result<String, String> {
        Ok(self.player(PlayerCommand::Resume))
    }

    #[tool(description = "Pula para a próxima faixa")]
    async fn player_proxima(&self) -> Result<String, String> {
        Ok(self.player(PlayerCommand::Next))
    }

    #[tool(description = "Volta para a faixa anterior")]
    async fn player_anterior(&self) -> Result<String, String> {
        Ok(self.player(PlayerCommand::Previous))
    }

    #[tool(description = "Define o volume do player do hub, de 0 a 100")]
    async fn player_volume(
        &self,
        Parameters(request): Parameters<VolumeRequest>,
    ) -> Result<String, String> {
        let volume = u8::try_from(request.volume)
            .ok()
            .filter(|v| *v <= 100)
            .ok_or("o volume vai de 0 a 100")?;
        Ok(self.player(PlayerCommand::SetVolume(volume)))
    }

    #[tool(description = "Para o player do hub e encerra a reprodução")]
    async fn player_parar(&self) -> Result<String, String> {
        Ok(self.player(PlayerCommand::Stop))
    }

    #[tool(description = "Lista os favoritos do player (links usados recentemente)")]
    async fn player_favoritos(&self) -> Result<String, String> {
        let favorites = player::favorites(self.ctx.db().conn()).map_err(err)?;
        let list: Vec<serde_json::Value> = favorites
            .iter()
            .map(|f| {
                json!({
                    "url": f.url,
                    "titulo": f.label(),
                    "playlist": f.is_playlist(),
                    "usado_em": f.used_at.format("%Y-%m-%d %H:%M").to_string(),
                })
            })
            .collect();
        Ok(text(&list))
    }

    #[tool(description = "Adiciona um link aos favoritos do player, ou remove com remover = true")]
    async fn player_favoritar(
        &self,
        Parameters(request): Parameters<FavoriteRequest>,
    ) -> Result<String, String> {
        let url = request.url.trim().to_string();
        if !player::is_supported_link(&url) {
            return Err("informe um link http ou https".to_string());
        }
        let removed = request.remover.unwrap_or(false);
        {
            let db = self.ctx.db();
            if removed {
                player::remove_favorite(db.conn(), &url).map_err(err)?;
            } else {
                player::touch_favorite(db.conn(), &url, self.ctx.clock.now()).map_err(err)?;
                if let Some(title) = filled(request.titulo) {
                    player::name_favorite(db.conn(), &url, &title).map_err(err)?;
                }
            }
        }
        self.ctx
            .events
            .publish(HubEvent::PlayerControl(PlayerCommand::FavoritesChanged));
        Ok(if removed {
            "Removido dos favoritos".to_string()
        } else {
            "Adicionado aos favoritos".to_string()
        })
    }

    #[tool(
        description = "Lista as conversas do WhatsApp guardadas no hub, com apelido, nome, se é grupo, se está arquivada e a última mensagem"
    )]
    async fn whatsapp_conversas(
        &self,
        Parameters(request): Parameters<ConversationsRequest>,
    ) -> Result<String, String> {
        let limit = request.limite.unwrap_or(30).clamp(1, 200);
        let query = request.busca.unwrap_or_default();
        let conversations = whatsapp::conversations(self.ctx.db().conn()).map_err(err)?;
        let list: Vec<serde_json::Value> = conversations
            .iter()
            .filter(|c| {
                request
                    .arquivadas
                    .is_none_or(|archived| c.archived == archived)
            })
            .filter(|c| c.matches(&query))
            .take(limit)
            .map(|c| {
                json!({
                    "jid": c.jid,
                    "titulo": c.title(),
                    "nome": c.name,
                    "apelido": c.alias,
                    "grupo": c.group,
                    "arquivada": c.archived,
                    "ultima_em": c.last_at.map(|at| at.format("%Y-%m-%d %H:%M").to_string()),
                    "previa": c.preview,
                })
            })
            .collect();
        Ok(text(&list))
    }

    #[tool(
        description = "Define o apelido de um contato ou grupo do WhatsApp no hub; texto vazio remove o apelido"
    )]
    async fn whatsapp_apelido(
        &self,
        Parameters(request): Parameters<AliasRequest>,
    ) -> Result<String, String> {
        let alias = request.apelido.trim().to_string();
        {
            let db = self.ctx.db();
            let conversation = whatsapp::find_conversation(db.conn(), &request.conversa)
                .map_err(err)?
                .ok_or("conversa não encontrada no hub")?;
            whatsapp::set_alias(db.conn(), &conversation.jid, &alias).map_err(err)?;
        }
        self.ctx.events.publish(HubEvent::WhatsAppChanged);
        Ok(if alias.is_empty() {
            "Apelido removido".to_string()
        } else {
            format!("Apelido definido: {alias}")
        })
    }

    #[tool(
        description = "Executa um comando no terminal do computador (PowerShell no Windows, sh no Linux) e devolve código de saída, stdout e stderr. Cada comando pede confirmação ao usuário no hub"
    )]
    async fn terminal_executar(
        &self,
        Parameters(request): Parameters<TerminalRequest>,
    ) -> Result<String, String> {
        let command = request.comando.trim().to_string();
        if command.is_empty() {
            return Err("o comando está vazio".to_string());
        }
        let folder = filled(request.pasta).unwrap_or_else(|| "~".to_string());
        let cwd = shortcuts::expand_home(&folder);
        if !cwd.is_dir() {
            return Err(format!("a pasta {} não existe", cwd.display()));
        }
        let timeout = terminal::clamp_timeout(request.tempo_limite_segundos);
        let os = self.ctx.platform.os();
        let pwsh = os == Os::Windows && terminal::has_pwsh();
        let approved = self
            .confirm(
                "Executar comando no terminal?".to_string(),
                format!(
                    "Pasta: {}\nShell: {}\nTempo limite: {} s\n\n{command}",
                    cwd.display(),
                    terminal::shell_name(os, pwsh),
                    timeout.as_secs()
                ),
            )
            .await;
        if !approved {
            return Err(REFUSED.to_string());
        }
        let spec = terminal::terminal_command(os, &command, pwsh);
        let output = terminal::run_terminal(&spec, &cwd, timeout)
            .await
            .map_err(|e| format!("não foi possível iniciar o terminal: {e}"))?;
        Ok(text(&json!({
            "codigo": output.exit_code,
            "tempo_esgotado": output.timed_out,
            "stdout": output.stdout,
            "stderr": output.stderr,
            "stdout_cortado": output.stdout_truncated,
            "stderr_cortado": output.stderr_truncated,
            "pasta": cwd.display().to_string(),
        })))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HubTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Ferramentas do hub pessoal do usuário: agenda (tarefas, rotinas, lembretes), notas, scripts, atalhos, player, WhatsApp e terminal (cada comando pede confirmação ao usuário). Datas no fuso local.",
        )
    }
}
