use std::sync::Arc;

use hub_core::agenda;
use hub_core::chrono::NaiveDateTime;
use hub_core::clock::FakeClock;
use hub_core::events::HubEvent;
use hub_core::platform::{FakePlatform, Os};
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
    _temp: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    Config::default().save(&paths.config_file()).unwrap();
    let ctx = HubContext::new(
        paths,
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 14:00")),
    );
    let token = ensure_token(&ctx).unwrap();
    let server = start(ctx.clone(), 0, token).unwrap();
    Fixture {
        ctx,
        server,
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
