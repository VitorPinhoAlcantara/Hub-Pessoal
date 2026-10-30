use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

pub const API_PATH: &str = "/api";
pub const URL_ENV: &str = "HUB_API";
pub const TOKEN_ENV: &str = "HUB_TOKEN";

#[derive(Clone)]
pub struct ApiState {
    pub mcp_url: String,
    pub token: String,
}

async fn connect(state: &ApiState) -> Result<RunningService<RoleClient, ()>, String> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(state.mcp_url.clone())
            .auth_header(state.token.clone()),
    );
    ().serve(transport).await.map_err(|e| e.to_string())
}

fn failure(status: StatusCode, message: String) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "erro": message })))
}

pub fn docs(tools: &[Tool], base: &str) -> String {
    let mut text = format!(
        "# API do hub\n\nCada ferramenta do MCP também é um endpoint: POST {base}/<ferramenta> com corpo JSON (os mesmos parâmetros) e o cabeçalho Authorization: Bearer $env:{TOKEN_ENV}. A resposta é {{\"resultado\": ...}} ou {{\"erro\": \"...\"}}. Ações sensíveis continuam pedindo confirmação no hub.\n\nExemplo em PowerShell:\n\n    $h = @{{ Authorization = \"Bearer $env:{TOKEN_ENV}\" }}\n    $tarefas = (Invoke-RestMethod -Method Post -Uri \"$env:{URL_ENV}/agenda_listar\" -Headers $h -ContentType 'application/json' -Body '{{}}').resultado\n\n## Ferramentas\n"
    );
    for tool in tools {
        let params = tool
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| props.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        text.push_str(&format!(
            "\n- {}({params}): {}",
            tool.name,
            tool.description.as_deref().unwrap_or_default()
        ));
    }
    text.push('\n');
    text
}

pub async fn index(State(state): State<ApiState>) -> Result<String, (StatusCode, Json<Value>)> {
    let client = connect(&state)
        .await
        .map_err(|e| failure(StatusCode::BAD_GATEWAY, e))?;
    let tools = client
        .list_all_tools()
        .await
        .map_err(|e| failure(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let _ = client.cancel().await;
    let base = state.mcp_url.replace(crate::MCP_PATH, API_PATH);
    Ok(docs(&tools, &base))
}

pub async fn call(
    State(state): State<ApiState>,
    Path(tool): Path<String>,
    body: Option<Json<Value>>,
) -> (StatusCode, Json<Value>) {
    let arguments = match body.map(|Json(value)| value) {
        None | Some(Value::Null) => Default::default(),
        Some(Value::Object(map)) => map,
        Some(_) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "o corpo precisa ser um objeto JSON".into(),
            );
        }
    };
    let client = match connect(&state).await {
        Ok(client) => client,
        Err(error) => return failure(StatusCode::BAD_GATEWAY, error),
    };
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await;
    let _ = client.cancel().await;
    let result = match result {
        Ok(result) => result,
        Err(error) => return failure(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error.unwrap_or(false) {
        return failure(StatusCode::UNPROCESSABLE_ENTITY, text);
    }
    let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (StatusCode::OK, Json(json!({ "resultado": value })))
}
