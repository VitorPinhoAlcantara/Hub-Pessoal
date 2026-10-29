use std::process::Stdio;
use std::time::Duration;

use hub_core::config::{AiConfig, AiProviderKind};
use hub_core::platform::{self, CommandSpec};

const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Checking,
    Ready(String),
    Missing(String),
    Problem(String),
}

impl Availability {
    pub fn usable(&self) -> bool {
        !matches!(self, Availability::Missing(_))
    }
}

pub fn install_hint(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Claude => {
            "Instale o Claude Code (npm install -g @anthropic-ai/claude-code) e faça login abrindo claude no terminal."
        }
        AiProviderKind::Codex => {
            "Instale com npm install -g @openai/codex (precisa do Node.js) e faça login com codex login."
        }
        AiProviderKind::Gemini => {
            "Instale com npm install -g @google/gemini-cli (precisa do Node.js) e faça login abrindo gemini no terminal."
        }
        AiProviderKind::Api => "Informe o endereço, o modelo e, se o serviço pedir, a chave.",
    }
}

async fn run(spec: &CommandSpec, args: &[&str]) -> Option<(bool, String)> {
    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    platform::hide_console(&mut command);
    let output = tokio::time::timeout(CHECK_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some((output.status.success(), text))
}

pub async fn check(kind: AiProviderKind, ai: AiConfig) -> Availability {
    let spec = match kind {
        AiProviderKind::Claude => CommandSpec::new("claude"),
        AiProviderKind::Codex => platform::npm_cli("codex"),
        AiProviderKind::Gemini => platform::npm_cli("gemini"),
        AiProviderKind::Api => return check_api(&ai).await,
    };
    let version = match run(&spec, &["--version"]).await {
        Some((true, text)) => text.lines().next().unwrap_or_default().to_string(),
        _ => return Availability::Missing("Não instalado".to_string()),
    };
    let logged_in = match kind {
        AiProviderKind::Claude => run(&spec, &["auth", "status"])
            .await
            .map(|(ok, text)| ok && !text.contains("\"loggedIn\": false")),
        AiProviderKind::Codex => run(&spec, &["login", "status"]).await.map(|(ok, _)| ok),
        _ => Some(true),
    };
    match logged_in {
        Some(false) => Availability::Problem(format!("Instalado ({version}), mas sem login")),
        _ => Availability::Ready(format!("Instalado ({version})")),
    }
}

async fn check_api(ai: &AiConfig) -> Availability {
    let url = ai.api_url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Availability::Problem("Informe o endereço da API".to_string());
    }
    let mut request = reqwest::Client::new()
        .get(format!("{url}/models"))
        .timeout(Duration::from_secs(8));
    if !ai.api_key.trim().is_empty() {
        request = request.bearer_auth(ai.api_key.trim());
    }
    match request.send().await {
        Err(_) => Availability::Problem(format!(
            "Sem resposta de {url} (o Ollama ou o LM Studio estão abertos?)"
        )),
        Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
            Availability::Problem("O serviço recusou a chave da API".to_string())
        }
        Ok(_) if ai.api_model.trim().is_empty() => {
            Availability::Problem("Servidor respondeu, falta informar o modelo".to_string())
        }
        Ok(_) => Availability::Ready(format!("Servidor respondeu, modelo {}", ai.api_model.trim())),
    }
}
