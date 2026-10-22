use std::sync::{Arc, Mutex};

use hub_core::agenda;
use hub_core::chrono::NaiveDateTime;
use hub_core::clock::FakeClock;
use hub_core::confirm::ConfirmRequest;
use hub_core::events::HubEvent;
use hub_core::platform::{FakePlatform, Os, PlatformCall};
use hub_core::player::PlayerCommand;
use hub_core::scripts::{self, NewScript};
use hub_core::{Config, Database, HubContext, HubPaths};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

use super::*;

struct Fixture {
    ctx: HubContext,
    server: McpServer,
    platform: Arc<FakePlatform>,
    _temp: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    Config::default().save(&paths.config_file()).unwrap();
    let platform = Arc::new(FakePlatform::default());
    let ctx = HubContext::new(
        paths,
        Config::default(),
        Database::open_in_memory().unwrap(),
        platform.clone(),
        Arc::new(FakeClock::at("2026-10-21 14:00")),
    );
    let token = ensure_token(&ctx).unwrap();
    let server = start(ctx.clone(), 0, token).unwrap();
    Fixture {
        ctx,
        server,
        platform,
        _temp: temp,
    }
}

async fn client(server: &McpServer, token: &str) -> RunningService<rmcp::RoleClient, ()> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(server.url()).auth_header(token.to_string()),
    );
    ().serve(transport).await.expect("cliente MCP conectado")
}

async fn call(
    client: &RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> CallToolResult {
    let arguments = args.as_object().cloned().unwrap_or_default();
    client
        .call_tool(CallToolRequestParams::new(name).with_arguments(arguments))
        .await
        .expect("chamada de ferramenta")
}

fn result_text(result: &CallToolResult) -> String {
    serde_json::to_value(&result.content)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()).map(str::to_string))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_error(result: &CallToolResult) -> bool {
    result.is_error.unwrap_or(false)
}

#[test]
fn token_is_generated_once_and_saved() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    let ctx = HubContext::new(
        paths.clone(),
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 14:00")),
    );
    let first = ensure_token(&ctx).unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(ensure_token(&ctx).unwrap(), first);
    assert_eq!(Config::load(&paths.config_file()).unwrap().mcp.token, first);
}

#[test]
fn client_config_points_to_server_with_token() {
    let value = client_config("http://127.0.0.1:1/mcp", "abc");
    assert_eq!(value["mcpServers"]["hub"]["url"], "http://127.0.0.1:1/mcp");
    assert_eq!(
        value["mcpServers"]["hub"]["headers"]["Authorization"],
        "Bearer abc"
    );
}

#[test]
fn moments_accept_iso_and_portuguese() {
    let now = NaiveDateTime::parse_from_str("2026-10-21 14:00", "%Y-%m-%d %H:%M").unwrap();
    assert_eq!(
        tools::parse_moment("2026-10-22 09:30", now)
            .unwrap()
            .to_string(),
        "2026-10-22 09:30:00"
    );
    assert_eq!(
        tools::parse_moment("em 1 minuto", now).unwrap().to_string(),
        "2026-10-21 14:01:00"
    );
    assert!(tools::parse_moment("algum dia", now).is_none());
    assert_eq!(
        tools::parse_date("25/12/2026", now).unwrap().to_string(),
        "2026-12-25"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_token_is_rejected() {
    let fixture = fixture();
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(fixture.server.url()).auth_header("errado"),
    );
    assert!(().serve(transport).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_all_tools() {
    let fixture = fixture();
    let client = client(&fixture.server, &fixture.server.token).await;
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    for expected in [
        "agenda_concluir",
        "agenda_criar_rotina",
        "agenda_criar_tarefa",
        "agenda_listar",
        "atalhos_adicionar",
        "atalhos_listar",
        "lembrete_criar",
        "player_tocar",
        "scripts_agendar",
        "scripts_criar",
        "scripts_executar",
        "scripts_listar",
        "whatsapp_enviar",
        "whatsapp_ler",
        "agenda_editar_tarefa",
        "agenda_mover",
        "agenda_excluir",
        "agenda_reabrir",
        "rotinas_listar",
        "rotinas_editar",
        "rotinas_ativar",
        "rotinas_excluir",
        "lembretes_listar",
        "lembrete_editar",
        "lembrete_excluir",
        "scripts_ler",
        "scripts_editar",
        "scripts_excluir",
        "scripts_historico",
        "atalhos_editar",
        "atalhos_excluir",
        "atalhos_abrir",
        "player_pausar",
        "player_continuar",
        "player_proxima",
        "player_anterior",
        "player_volume",
        "player_parar",
        "player_favoritos",
        "player_favoritar",
        "whatsapp_conversas",
        "whatsapp_apelido",
        "terminal_executar",
    ] {
        assert!(names.contains(&expected.to_string()), "faltou {expected}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn reminder_from_natural_language_reaches_agenda() {
    let fixture = fixture();
    let mut events = fixture.ctx.events.subscribe();
    let client = client(&fixture.server, &fixture.server.token).await;
    let result = call(
        &client,
        "lembrete_criar",
        json!({ "titulo": "beber água", "quando": "em 1 minuto" }),
    )
    .await;
    assert!(!is_error(&result), "{}", result_text(&result));
    assert!(result_text(&result).contains("2026-10-21 14:01"));
    assert_eq!(
        agenda::next_reminder_time(fixture.ctx.db().conn()).unwrap(),
        Some(NaiveDateTime::parse_from_str("2026-10-21 14:01", "%Y-%m-%d %H:%M").unwrap())
    );
    assert_eq!(events.recv().await.unwrap(), HubEvent::AgendaChanged);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn agenda_tools_create_list_and_complete() {
    let fixture = fixture();
    let client = client(&fixture.server, &fixture.server.token).await;
    let created = call(
        &client,
        "agenda_criar_tarefa",
        json!({ "titulo": "Reunião", "data": "amanhã", "hora": "10:00", "prioridade": "alta" }),
    )
    .await;
    assert!(!is_error(&created), "{}", result_text(&created));
    let id = serde_json::from_str::<Value>(&result_text(&created)).unwrap()["criada"]
        .as_i64()
        .unwrap();
    let routine = call(
        &client,
        "agenda_criar_rotina",
        json!({ "titulo": "Remédio", "recorrencia": "diaria", "hora": "20:00" }),
    )
    .await;
    assert!(!is_error(&routine), "{}", result_text(&routine));
    let listed = call(&client, "agenda_listar", json!({})).await;
    let text = result_text(&listed);
    assert!(text.contains("Reunião"));
    assert!(text.contains("Remédio"));
    let done = call(&client, "agenda_concluir", json!({ "id": id })).await;
    assert!(!is_error(&done));
    assert!(
        agenda::get_task(fixture.ctx.db().conn(), id)
            .unwrap()
            .is_completed()
    );
    let bad = call(
        &client,
        "agenda_criar_rotina",
        json!({ "titulo": "x", "recorrencia": "às vezes" }),
    )
    .await;
    assert!(is_error(&bad));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn created_scripts_start_inactive() {
    let fixture = fixture();
    let client = client(&fixture.server, &fixture.server.token).await;
    let result = call(
        &client,
        "scripts_criar",
        json!({
            "nome": "Limpar downloads",
            "arquivo_windows": "limpar.ps1",
            "conteudo_windows": "Write-Output ok",
            "arquivo_linux": "limpar.sh",
            "conteudo_linux": "echo ok"
        }),
    )
    .await;
    assert!(!is_error(&result), "{}", result_text(&result));
    let script = fixture.ctx.script("limpar-downloads").unwrap();
    assert!(!script.meta.active);
    let refused = call(
        &client,
        "scripts_executar",
        json!({ "id": "limpar-downloads" }),
    )
    .await;
    assert!(is_error(&refused));
    assert!(result_text(&refused).contains("inativo"));
    let listed = result_text(&call(&client, "scripts_listar", json!({})).await);
    assert!(listed.contains("limpar-downloads"));
    client.cancel().await.unwrap();
}

fn runnable_script(ctx: &HubContext) {
    let (windows, linux) = ("Write-Output 'feito'", "echo feito");
    scripts::create(
        &ctx.paths.scripts_dir,
        NewScript {
            name: "Rodar".into(),
            windows_file: Some("rodar.ps1".into()),
            linux_file: Some("rodar.sh".into()),
            windows_content: windows.into(),
            linux_content: linux.into(),
            active: true,
            ..NewScript::default()
        },
    )
    .unwrap();
    assert!(ctx.script("rodar").unwrap().file.is_some() || Os::current() == Os::Linux);
}

#[tokio::test(flavor = "multi_thread")]
async fn sensitive_tool_waits_for_confirmation() {
    let fixture = fixture();
    runnable_script(&fixture.ctx);
    let mut events = fixture.ctx.events.subscribe();
    let broker = fixture.ctx.confirmations.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let HubEvent::ConfirmationRequested(request) = event {
                broker.answer(request.id, true);
            }
        }
    });
    let client = client(&fixture.server, &fixture.server.token).await;
    let result = call(&client, "scripts_executar", json!({ "id": "rodar" })).await;
    assert!(!is_error(&result), "{}", result_text(&result));
    let text = result_text(&result);
    assert!(text.contains("feito"), "{text}");
    assert!(text.contains("Concluído"));
    let history = scripts::executions(fixture.ctx.db().conn(), "rodar", 5).unwrap();
    assert_eq!(history[0].origin, scripts::Origin::Ai);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_confirmation_blocks_schedule_change() {
    let fixture = fixture();
    runnable_script(&fixture.ctx);
    let mut events = fixture.ctx.events.subscribe();
    let broker = fixture.ctx.confirmations.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let HubEvent::ConfirmationRequested(request) = event {
                broker.answer(request.id, false);
            }
        }
    });
    let client = client(&fixture.server, &fixture.server.token).await;
    let result = call(
        &client,
        "scripts_agendar",
        json!({ "id": "rodar", "agendamento": "0 0 3 * * *" }),
    )
    .await;
    assert!(is_error(&result));
    assert!(fixture.ctx.script("rodar").unwrap().meta.schedule.is_none());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn shortcuts_and_player_tools() {
    let fixture = fixture();
    let mut events = fixture.ctx.events.subscribe();
    let client = client(&fixture.server, &fixture.server.token).await;
    let added = call(
        &client,
        "atalhos_adicionar",
        json!({ "nome": "GitHub", "tipo": "url", "alvo": "github.com", "grupo": "Dev" }),
    )
    .await;
    assert!(!is_error(&added), "{}", result_text(&added));
    assert!(result_text(&call(&client, "atalhos_listar", json!({})).await).contains("GitHub"));
    let bad = call(
        &client,
        "atalhos_adicionar",
        json!({ "nome": "x", "tipo": "foguete", "alvo": "y" }),
    )
    .await;
    assert!(is_error(&bad));
    let played = call(
        &client,
        "player_tocar",
        json!({ "url": "https://www.youtube.com/playlist?list=abc" }),
    )
    .await;
    assert!(!is_error(&played));
    let mut saw_play = false;
    while let Ok(event) = events.try_recv() {
        if let HubEvent::PlayRequested(url) = event {
            saw_play = url.contains("list=abc");
        }
    }
    assert!(saw_play);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn whatsapp_read_uses_stored_messages_and_send_needs_open_tab() {
    let fixture = fixture();
    {
        let db = fixture.ctx.db();
        let wire = hub_core::whatsapp::WireMessage {
            id: "1".into(),
            jid: "ana@s.whatsapp.net".into(),
            de: "ana@s.whatsapp.net".into(),
            nome_de: "Ana".into(),
            texto: "chego às 8".into(),
            minha: false,
            em: 1_792_627_000,
            tipo: "texto".into(),
            historico: false,
        };
        hub_core::whatsapp::store_message(db.conn(), &hub_core::whatsapp::wire_to_message(&wire))
            .unwrap();
        hub_core::whatsapp::set_alias(db.conn(), "ana@s.whatsapp.net", "Irmã").unwrap();
    }
    let client = client(&fixture.server, &fixture.server.token).await;
    let read = call(&client, "whatsapp_ler", json!({ "conversa": "irmã" })).await;
    assert!(!is_error(&read), "{}", result_text(&read));
    assert!(result_text(&read).contains("chego às 8"));
    let missing = call(&client, "whatsapp_ler", json!({ "conversa": "ninguém" })).await;
    assert!(is_error(&missing));
    let send = call(
        &client,
        "whatsapp_enviar",
        json!({ "conversa": "Irmã", "texto": "ok" }),
    )
    .await;
    assert!(is_error(&send));
    assert!(result_text(&send).contains("aba WhatsApp"));
    client.cancel().await.unwrap();
}

fn auto_answer(ctx: &HubContext, approved: bool) -> Arc<Mutex<Vec<ConfirmRequest>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut events = ctx.events.subscribe();
    let broker = ctx.confirmations.clone();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let HubEvent::ConfirmationRequested(request) = event {
                log.lock().unwrap().push(request.clone());
                broker.answer(request.id, approved);
            }
        }
    });
    seen
}

fn json_of(result: &CallToolResult) -> Value {
    serde_json::from_str(&result_text(result)).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn agenda_list_includes_routine_previews() {
    let fixture = fixture();
    let client = client(&fixture.server, &fixture.server.token).await;
    let routine = call(
        &client,
        "agenda_criar_rotina",
        json!({ "titulo": "Academia", "recorrencia": "diaria", "hora": "07:00" }),
    )
    .await;
    assert!(!is_error(&routine), "{}", result_text(&routine));
    let listed = json_of(
        &call(
            &client,
            "agenda_listar",
            json!({ "de": "2026-10-21", "ate": "2026-10-23" }),
        )
        .await,
    );
    let items = listed["itens"].as_array().unwrap();
    let previews: Vec<&Value> = items
        .iter()
        .filter(|i| i["tipo"] == "previsao_rotina")
        .collect();
    assert_eq!(previews.len(), 2);
    assert_eq!(previews[0]["data"], "2026-10-22");
    assert_eq!(previews[0]["previsao"], true);
    assert_eq!(previews[0]["hora"], "07:00");
    assert!(
        items
            .iter()
            .any(|i| i["tipo"] == "rotina" && i["data"] == "2026-10-21")
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn agenda_tasks_can_be_edited_moved_reopened_and_deleted() {
    let fixture = fixture();
    let client = client(&fixture.server, &fixture.server.token).await;
    let created = json_of(
        &call(
            &client,
            "agenda_criar_tarefa",
            json!({ "titulo": "Dentista", "data": "2026-10-22", "hora": "09:00" }),
        )
        .await,
    );
    let id = created["criada"].as_i64().unwrap();
    let edited = call(
        &client,
        "agenda_editar_tarefa",
        json!({ "id": id, "titulo": "Dentista Dr. Paulo", "hora": "10:30", "prioridade": "alta", "notas": "levar exames" }),
    )
    .await;
    assert!(!is_error(&edited), "{}", result_text(&edited));
    let task = agenda::get_task(fixture.ctx.db().conn(), id).unwrap();
    assert_eq!(task.title, "Dentista Dr. Paulo");
    assert_eq!(task.time.unwrap().to_string(), "10:30:00");
    assert_eq!(task.date.unwrap().to_string(), "2026-10-22");
    assert_eq!(task.notes, "levar exames");
    let moved = call(
        &client,
        "agenda_mover",
        json!({ "id": id, "data": "25/10/2026" }),
    )
    .await;
    assert!(!is_error(&moved), "{}", result_text(&moved));
    let task = agenda::get_task(fixture.ctx.db().conn(), id).unwrap();
    assert_eq!(task.date.unwrap().to_string(), "2026-10-25");
    assert_eq!(task.time.unwrap().to_string(), "10:30:00");
    let cleared = call(
        &client,
        "agenda_editar_tarefa",
        json!({ "id": id, "hora": "" }),
    )
    .await;
    assert!(!is_error(&cleared));
    assert!(
        agenda::get_task(fixture.ctx.db().conn(), id)
            .unwrap()
            .time
            .is_none()
    );
    call(&client, "agenda_concluir", json!({ "id": id })).await;
    let reopened = call(&client, "agenda_reabrir", json!({ "id": id })).await;
    assert!(!is_error(&reopened));
    assert!(
        !agenda::get_task(fixture.ctx.db().conn(), id)
            .unwrap()
            .is_completed()
    );
    let bad = call(
        &client,
        "agenda_editar_tarefa",
        json!({ "id": id, "hora": "25h99" }),
    )
    .await;
    assert!(is_error(&bad));
    let deleted = call(&client, "agenda_excluir", json!({ "id": id })).await;
    assert!(!is_error(&deleted), "{}", result_text(&deleted));
    assert!(agenda::get_task(fixture.ctx.db().conn(), id).is_err());
    let missing = call(&client, "agenda_excluir", json!({ "id": id })).await;
    assert!(is_error(&missing));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn reminders_can_be_listed_edited_moved_and_deleted() {
    let fixture = fixture();
    let client = client(&fixture.server, &fixture.server.token).await;
    let created = json_of(
        &call(
            &client,
            "lembrete_criar",
            json!({ "titulo": "Pagar conta", "quando": "2026-10-22 08:00" }),
        )
        .await,
    );
    let id = created["criado"].as_i64().unwrap();
    let listed = json_of(&call(&client, "lembretes_listar", json!({})).await);
    assert_eq!(listed[0]["titulo"], "Pagar conta");
    assert_eq!(listed[0]["quando"], "2026-10-22 08:00");
    let edited = call(
        &client,
        "lembrete_editar",
        json!({ "id": id, "titulo": "Pagar luz", "quando": "2026-10-22 18:15", "notas": "app do banco" }),
    )
    .await;
    assert!(!is_error(&edited), "{}", result_text(&edited));
    let moved = call(
        &client,
        "agenda_mover",
        json!({ "id": id, "tipo": "lembrete", "data": "2026-10-24" }),
    )
    .await;
    assert!(!is_error(&moved), "{}", result_text(&moved));
    let reminder = agenda::get_reminder(fixture.ctx.db().conn(), id).unwrap();
    assert_eq!(reminder.title, "Pagar luz");
    assert_eq!(reminder.notes, "app do banco");
    assert_eq!(reminder.effective_time().to_string(), "2026-10-24 18:15:00");
    let no_date = call(
        &client,
        "agenda_mover",
        json!({ "id": id, "tipo": "lembrete", "data": "" }),
    )
    .await;
    assert!(is_error(&no_date));
    let bad_kind = call(
        &client,
        "agenda_excluir",
        json!({ "id": id, "tipo": "foguete" }),
    )
    .await;
    assert!(is_error(&bad_kind));
    let deleted = call(&client, "lembrete_excluir", json!({ "id": id })).await;
    assert!(!is_error(&deleted));
    assert!(agenda::get_reminder(fixture.ctx.db().conn(), id).is_err());
    let other = json_of(
        &call(
            &client,
            "lembrete_criar",
            json!({ "titulo": "Outro", "quando": "em 10 minutos" }),
        )
        .await,
    );
    let other = other["criado"].as_i64().unwrap();
    let via_agenda = call(
        &client,
        "agenda_excluir",
        json!({ "id": other, "tipo": "lembrete" }),
    )
    .await;
    assert!(!is_error(&via_agenda));
    assert_eq!(
        json_of(&call(&client, "lembretes_listar", json!({})).await),
        json!([])
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn routines_can_be_listed_edited_paused_and_deleted() {
    let fixture = fixture();
    let client = client(&fixture.server, &fixture.server.token).await;
    let created = json_of(
        &call(
            &client,
            "agenda_criar_rotina",
            json!({ "titulo": "Remédio", "recorrencia": "diaria" }),
        )
        .await,
    );
    let id = created["criada"].as_i64().unwrap();
    let listed = json_of(&call(&client, "rotinas_listar", json!({})).await);
    assert_eq!(listed[0]["titulo"], "Remédio");
    assert_eq!(listed[0]["ativa"], true);
    let edited = call(
        &client,
        "rotinas_editar",
        json!({ "id": id, "recorrencia": "semana:1,3,5", "hora": "21:00", "titulo": "Vitamina" }),
    )
    .await;
    assert!(!is_error(&edited), "{}", result_text(&edited));
    let routine = agenda::get_routine(fixture.ctx.db().conn(), id).unwrap();
    assert_eq!(routine.title, "Vitamina");
    assert_eq!(routine.recurrence.to_string(), "semana:1,3,5");
    assert_eq!(routine.time.unwrap().to_string(), "21:00:00");
    let bad = call(
        &client,
        "rotinas_editar",
        json!({ "id": id, "recorrencia": "quando der" }),
    )
    .await;
    assert!(is_error(&bad));
    let paused = call(
        &client,
        "rotinas_ativar",
        json!({ "id": id, "ativa": false }),
    )
    .await;
    assert!(!is_error(&paused));
    assert!(
        !agenda::get_routine(fixture.ctx.db().conn(), id)
            .unwrap()
            .active
    );
    let deleted = call(&client, "rotinas_excluir", json!({ "id": id })).await;
    assert!(!is_error(&deleted));
    assert!(
        agenda::list_routines(fixture.ctx.db().conn())
            .unwrap()
            .is_empty()
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn scripts_can_be_read_edited_and_show_history() {
    let fixture = fixture();
    runnable_script(&fixture.ctx);
    let client = client(&fixture.server, &fixture.server.token).await;
    let read = json_of(&call(&client, "scripts_ler", json!({ "id": "rodar" })).await);
    assert_eq!(read["codigo_windows"], "Write-Output 'feito'");
    assert_eq!(read["codigo_linux"], "echo feito");
    assert_eq!(read["arquivo_linux"], "rodar.sh");
    let renamed = call(
        &client,
        "scripts_editar",
        json!({ "id": "rodar", "descricao": "Roda algo", "nome": "Rodar tudo" }),
    )
    .await;
    assert!(!is_error(&renamed), "{}", result_text(&renamed));
    assert_eq!(json_of(&renamed)["pediu_confirmacao"], false);
    let script = fixture.ctx.script("rodar").unwrap();
    assert_eq!(script.meta.description, "Roda algo");
    assert_eq!(script.meta.name, "Rodar tudo");
    let nothing = call(&client, "scripts_editar", json!({ "id": "rodar" })).await;
    assert!(is_error(&nothing));
    let escape = call(
        &client,
        "scripts_editar",
        json!({ "id": "rodar", "arquivo_linux": "../fora.sh" }),
    )
    .await;
    assert!(is_error(&escape));
    {
        let db = fixture.ctx.db();
        let started = NaiveDateTime::parse_from_str("2026-10-21 13:00", "%Y-%m-%d %H:%M").unwrap();
        let run =
            scripts::start_execution(db.conn(), "rodar", scripts::Origin::Manual, started).unwrap();
        scripts::finish_execution(db.conn(), run, Some(0), "tudo certo\n", started).unwrap();
    }
    let history = json_of(&call(&client, "scripts_historico", json!({ "id": "rodar" })).await);
    assert_eq!(history[0]["codigo"], 0);
    assert_eq!(history[0]["sucesso"], true);
    assert!(history[0]["saida"].as_str().unwrap().contains("tudo certo"));
    let all = json_of(&call(&client, "scripts_historico", json!({})).await);
    assert_eq!(all.as_array().unwrap().len(), 1);
    let missing = call(&client, "scripts_historico", json!({ "id": "nao-existe" })).await;
    assert!(is_error(&missing));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn script_code_change_shows_new_code_in_confirmation() {
    let fixture = fixture();
    runnable_script(&fixture.ctx);
    let seen = auto_answer(&fixture.ctx, true);
    let client = client(&fixture.server, &fixture.server.token).await;
    let edited = call(
        &client,
        "scripts_editar",
        json!({ "id": "rodar", "conteudo_linux": "echo novo codigo", "agendamento": "0 0 3 * * *" }),
    )
    .await;
    assert!(!is_error(&edited), "{}", result_text(&edited));
    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].detail.contains("echo novo codigo"));
    assert!(requests[0].detail.contains("0 0 3 * * *"));
    let script = fixture.ctx.script("rodar").unwrap();
    assert_eq!(script.meta.schedule.as_deref(), Some("0 0 3 * * *"));
    assert_eq!(
        std::fs::read_to_string(fixture.ctx.paths.scripts_dir.join("rodar.sh")).unwrap(),
        "echo novo codigo"
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_script_edit_and_delete_change_nothing() {
    let fixture = fixture();
    runnable_script(&fixture.ctx);
    let seen = auto_answer(&fixture.ctx, false);
    let client = client(&fixture.server, &fixture.server.token).await;
    let edited = call(
        &client,
        "scripts_editar",
        json!({ "id": "rodar", "conteudo_windows": "Write-Output 'alterado'" }),
    )
    .await;
    assert!(is_error(&edited));
    assert_eq!(
        std::fs::read_to_string(fixture.ctx.paths.scripts_dir.join("rodar.ps1")).unwrap(),
        "Write-Output 'feito'"
    );
    let deleted = call(&client, "scripts_excluir", json!({ "id": "rodar" })).await;
    assert!(is_error(&deleted));
    assert!(fixture.ctx.script("rodar").is_ok());
    assert_eq!(seen.lock().unwrap().len(), 2);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn approved_script_delete_removes_files() {
    let fixture = fixture();
    runnable_script(&fixture.ctx);
    let seen = auto_answer(&fixture.ctx, true);
    let client = client(&fixture.server, &fixture.server.token).await;
    let deleted = call(&client, "scripts_excluir", json!({ "id": "rodar" })).await;
    assert!(!is_error(&deleted), "{}", result_text(&deleted));
    assert!(seen.lock().unwrap()[0].detail.contains("rodar.sh"));
    assert!(fixture.ctx.script("rodar").is_err());
    assert!(!fixture.ctx.paths.scripts_dir.join("rodar.ps1").exists());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn shortcuts_can_be_edited_opened_and_deleted() {
    let fixture = fixture();
    let seen = auto_answer(&fixture.ctx, false);
    let client = client(&fixture.server, &fixture.server.token).await;
    for (name, kind, target) in [
        ("GitHub", "url", "github.com"),
        ("Limpar", "comando", "echo limpar"),
    ] {
        let added = call(
            &client,
            "atalhos_adicionar",
            json!({ "nome": name, "tipo": kind, "alvo": target }),
        )
        .await;
        assert!(!is_error(&added), "{}", result_text(&added));
    }
    let edited = call(
        &client,
        "atalhos_editar",
        json!({ "indice": 0, "nome": "Repositórios", "grupo": "Dev" }),
    )
    .await;
    assert!(!is_error(&edited), "{}", result_text(&edited));
    let listed = json_of(&call(&client, "atalhos_listar", json!({})).await);
    assert_eq!(listed["atalhos"][0]["nome"], "Repositórios");
    assert_eq!(listed["atalhos"][0]["grupo"], "Dev");
    assert_eq!(listed["atalhos"][0]["tipo"], "url");
    assert_eq!(listed["atalhos"][0]["alvo_linux"], "github.com");
    let opened = call(&client, "atalhos_abrir", json!({ "nome": "repositórios" })).await;
    assert!(!is_error(&opened), "{}", result_text(&opened));
    assert_eq!(
        fixture.platform.calls(),
        vec![PlatformCall::OpenUrl("https://github.com".into())]
    );
    let command = call(&client, "atalhos_abrir", json!({ "indice": 1 })).await;
    assert!(is_error(&command));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(seen.lock().unwrap()[0].detail.contains("echo limpar"));
    assert_eq!(fixture.platform.calls().len(), 1);
    let missing = call(&client, "atalhos_abrir", json!({ "nome": "zzz" })).await;
    assert!(is_error(&missing));
    let removed = call(&client, "atalhos_excluir", json!({ "indice": 1 })).await;
    assert!(!is_error(&removed));
    let listed = json_of(&call(&client, "atalhos_listar", json!({})).await);
    assert_eq!(listed["atalhos"].as_array().unwrap().len(), 1);
    let out_of_range = call(&client, "atalhos_excluir", json!({ "indice": 7 })).await;
    assert!(is_error(&out_of_range));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn player_controls_publish_commands_and_favorites_are_stored() {
    let fixture = fixture();
    let mut events = fixture.ctx.events.subscribe();
    let client = client(&fixture.server, &fixture.server.token).await;
    for name in [
        "player_pausar",
        "player_continuar",
        "player_proxima",
        "player_anterior",
        "player_parar",
    ] {
        let result = call(&client, name, json!({})).await;
        assert!(!is_error(&result), "{name}");
    }
    let volume = call(&client, "player_volume", json!({ "volume": 35 })).await;
    assert!(!is_error(&volume));
    let loud = call(&client, "player_volume", json!({ "volume": 150 })).await;
    assert!(is_error(&loud));
    let mut commands = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let HubEvent::PlayerControl(command) = event {
            commands.push(command);
        }
    }
    assert_eq!(
        commands,
        vec![
            PlayerCommand::Pause,
            PlayerCommand::Resume,
            PlayerCommand::Next,
            PlayerCommand::Previous,
            PlayerCommand::Stop,
            PlayerCommand::SetVolume(35),
        ]
    );
    let saved = call(
        &client,
        "player_favoritar",
        json!({ "url": "https://youtu.be/lofi", "titulo": "Lo-fi" }),
    )
    .await;
    assert!(!is_error(&saved), "{}", result_text(&saved));
    let listed = json_of(&call(&client, "player_favoritos", json!({})).await);
    assert_eq!(listed[0]["titulo"], "Lo-fi");
    assert_eq!(listed[0]["url"], "https://youtu.be/lofi");
    let invalid = call(&client, "player_favoritar", json!({ "url": "youtu.be/x" })).await;
    assert!(is_error(&invalid));
    let removed = call(
        &client,
        "player_favoritar",
        json!({ "url": "https://youtu.be/lofi", "remover": true }),
    )
    .await;
    assert!(!is_error(&removed));
    assert_eq!(
        json_of(&call(&client, "player_favoritos", json!({})).await),
        json!([])
    );
    let mut saw_favorites = false;
    while let Ok(event) = events.try_recv() {
        saw_favorites |= event == HubEvent::PlayerControl(PlayerCommand::FavoritesChanged);
    }
    assert!(saw_favorites);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn whatsapp_conversations_are_listed_and_aliased() {
    let fixture = fixture();
    {
        let db = fixture.ctx.db();
        for (id, jid, name, text) in [
            ("1", "ana@s.whatsapp.net", "Ana", "oi"),
            ("2", "beto@s.whatsapp.net", "Beto", "tudo bem?"),
        ] {
            let wire = hub_core::whatsapp::WireMessage {
                id: id.into(),
                jid: jid.into(),
                de: jid.into(),
                nome_de: name.into(),
                texto: text.into(),
                minha: false,
                em: 1_792_627_000,
                tipo: "texto".into(),
                historico: false,
            };
            hub_core::whatsapp::store_message(
                db.conn(),
                &hub_core::whatsapp::wire_to_message(&wire),
            )
            .unwrap();
        }
    }
    let mut events = fixture.ctx.events.subscribe();
    let client = client(&fixture.server, &fixture.server.token).await;
    let all = json_of(&call(&client, "whatsapp_conversas", json!({})).await);
    assert_eq!(all.as_array().unwrap().len(), 2);
    let filtered = json_of(&call(&client, "whatsapp_conversas", json!({ "busca": "beto" })).await);
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(filtered[0]["previa"], "tudo bem?");
    let aliased = call(
        &client,
        "whatsapp_apelido",
        json!({ "conversa": "Ana", "apelido": "Mãe" }),
    )
    .await;
    assert!(!is_error(&aliased), "{}", result_text(&aliased));
    let found = json_of(&call(&client, "whatsapp_conversas", json!({ "busca": "mãe" })).await);
    assert_eq!(found[0]["jid"], "ana@s.whatsapp.net");
    assert_eq!(found[0]["apelido"], "Mãe");
    assert_eq!(events.recv().await.unwrap(), HubEvent::WhatsAppChanged);
    let cleared = call(
        &client,
        "whatsapp_apelido",
        json!({ "conversa": "Mãe", "apelido": "" }),
    )
    .await;
    assert!(!is_error(&cleared));
    let missing = call(
        &client,
        "whatsapp_apelido",
        json!({ "conversa": "ninguém", "apelido": "x" }),
    )
    .await;
    assert!(is_error(&missing));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn terminal_runs_only_after_confirmation_showing_command() {
    let fixture = fixture();
    let seen = auto_answer(&fixture.ctx, true);
    let client = client(&fixture.server, &fixture.server.token).await;
    let folder = fixture._temp.path().to_path_buf();
    let result = call(
        &client,
        "terminal_executar",
        json!({ "comando": "echo ola; exit 2", "pasta": folder.display().to_string() }),
    )
    .await;
    assert!(!is_error(&result), "{}", result_text(&result));
    let value = json_of(&result);
    assert!(value["stdout"].as_str().unwrap().contains("ola"), "{value}");
    assert_eq!(value["codigo"], 2);
    assert_eq!(value["tempo_esgotado"], false);
    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].detail.contains("echo ola; exit 2"));
    assert!(requests[0].detail.contains(&folder.display().to_string()));
    let empty = call(&client, "terminal_executar", json!({ "comando": "  " })).await;
    assert!(is_error(&empty));
    let no_folder = call(
        &client,
        "terminal_executar",
        json!({ "comando": "echo x", "pasta": folder.join("nao-existe").display().to_string() }),
    )
    .await;
    assert!(is_error(&no_folder));
    assert_eq!(seen.lock().unwrap().len(), 1);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_terminal_command_does_not_run() {
    let fixture = fixture();
    let seen = auto_answer(&fixture.ctx, false);
    let client = client(&fixture.server, &fixture.server.token).await;
    let folder = fixture._temp.path().to_path_buf();
    let marker = folder.join("marcador.txt");
    let command = match Os::current() {
        Os::Windows => "New-Item -ItemType File marcador.txt",
        Os::Linux => "touch marcador.txt",
    };
    let result = call(
        &client,
        "terminal_executar",
        json!({ "comando": command, "pasta": folder.display().to_string() }),
    )
    .await;
    assert!(is_error(&result));
    assert!(result_text(&result).contains("recusou"));
    assert!(!marker.exists());
    assert_eq!(seen.lock().unwrap().len(), 1);
    client.cancel().await.unwrap();
}
