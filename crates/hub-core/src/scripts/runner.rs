use std::process::Stdio;
use std::time::Duration;

use futures::channel::mpsc;
use futures::{FutureExt, Stream, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::watch;

use super::{Origin, Script, finish_execution, start_execution};
use crate::context::HubContext;
use crate::events::HubEvent;
use crate::platform::{self, Notification};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Completed(i32),
    Failed(String),
    Cancelled,
    TimedOut,
}

impl Status {
    pub fn succeeded(&self) -> bool {
        *self == Status::Completed(0)
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Status::Completed(code) => Some(*code),
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Status::Completed(0) => "Concluído".to_string(),
            Status::Completed(code) => format!("Terminou com código {code}"),
            Status::Failed(reason) => format!("Falhou: {reason}"),
            Status::Cancelled => "Cancelado".to_string(),
            Status::TimedOut => "Tempo limite atingido".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    Started {
        execution_id: i64,
    },
    Line {
        stream: OutputStream,
        text: String,
    },
    Finished {
        execution_id: i64,
        status: Status,
        output: String,
    },
}

#[derive(Clone)]
pub struct CancelHandle(watch::Sender<bool>);

impl CancelHandle {
    pub fn cancel(&self) {
        let _ = self.0.send(true);
    }
}

impl std::fmt::Debug for CancelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CancelHandle")
    }
}

pub fn run(
    ctx: HubContext,
    script: Script,
    origin: Origin,
) -> (impl Stream<Item = RunEvent> + Send + 'static, CancelHandle) {
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (tx, rx) = mpsc::unbounded();
    let driver = drive(ctx, script, origin, cancel_rx, tx)
        .into_stream()
        .filter_map(|_| async { None });
    (futures::stream::select(driver, rx), CancelHandle(cancel_tx))
}

pub async fn run_to_end(ctx: HubContext, script: Script, origin: Origin) -> (Status, String) {
    let (events, _cancel) = run(ctx, script, origin);
    let mut events = Box::pin(events);
    while let Some(event) = events.next().await {
        if let RunEvent::Finished { status, output, .. } = event {
            return (status, output);
        }
    }
    (
        Status::Failed("execução interrompida".to_string()),
        String::new(),
    )
}

async fn read_line<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Option<String> {
    let mut buffer = Vec::new();
    match reader.read_until(b'\n', &mut buffer).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(
            String::from_utf8_lossy(&buffer)
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        ),
    }
}

fn push_output(output: &mut String, text: &str) {
    output.push_str(text);
    output.push('\n');
    if output.len() > super::OUTPUT_LIMIT * 2 {
        let keep = super::truncate_output(output).to_string();
        *output = keep;
    }
}

async fn drive(
    ctx: HubContext,
    script: Script,
    origin: Origin,
    mut cancel: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<RunEvent>,
) {
    let started = ctx.clock.now();
    let execution_id = {
        let db = ctx.db();
        match start_execution(db.conn(), &script.id, origin, started) {
            Ok(id) => id,
            Err(error) => {
                let _ = tx.unbounded_send(RunEvent::Finished {
                    execution_id: 0,
                    status: Status::Failed(error.to_string()),
                    output: String::new(),
                });
                return;
            }
        }
    };
    let _ = tx.unbounded_send(RunEvent::Started { execution_id });
    let mut output = String::new();
    let status = execute(&ctx, &script, &mut cancel, &tx, &mut output).await;
    if !matches!(status, Status::Completed(_)) {
        push_output(&mut output, &format!("[{}]", status.describe()));
    }
    {
        let db = ctx.db();
        if let Err(error) = finish_execution(
            db.conn(),
            execution_id,
            status.exit_code(),
            &output,
            ctx.clock.now(),
        ) {
            tracing::warn!(%error, "falha ao registrar o fim do script");
        }
    }
    tracing::info!(script = %script.id, origem = origin.as_str(), status = ?status, "script executado");
    if origin == Origin::Scheduled && !status.succeeded() {
        let _ = ctx.platform.notify(
            Notification {
                title: format!("Script {} falhou", script.meta.name),
                body: status.describe(),
                actions: Vec::new(),
            },
            None,
        );
    }
    ctx.events.publish(HubEvent::ScriptsChanged);
    let _ = tx.unbounded_send(RunEvent::Finished {
        execution_id,
        status,
        output: super::truncate_output(&output).to_string(),
    });
}

async fn execute(
    ctx: &HubContext,
    script: &Script,
    cancel: &mut watch::Receiver<bool>,
    tx: &mpsc::UnboundedSender<RunEvent>,
    output: &mut String,
) -> Status {
    let Some(file) = &script.file else {
        return Status::Failed(
            script
                .problem
                .clone()
                .unwrap_or_else(|| "sem arquivo para este sistema".to_string()),
        );
    };
    let spec = match ctx.platform.script_command(file) {
        Ok(spec) => spec,
        Err(error) => return Status::Failed(error.to_string()),
    };
    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(file.parent().unwrap_or(&ctx.paths.scripts_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    platform::hide_console(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Status::Failed(format!(
                "não foi possível iniciar {}: {error}",
                spec.program.to_string_lossy()
            ));
        }
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Status::Failed("sem acesso à saída do processo".to_string());
    };
    let mut stdout = BufReader::new(stdout);
    let mut stderr = BufReader::new(stderr);
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut cancel_alive = true;
    let timeout = tokio::time::sleep(Duration::from_secs(script.meta.timeout_secs.max(1)));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            line = read_line(&mut stdout), if stdout_open => match line {
                Some(text) => {
                    push_output(output, &text);
                    let _ = tx.unbounded_send(RunEvent::Line { stream: OutputStream::Stdout, text });
                }
                None => stdout_open = false,
            },
            line = read_line(&mut stderr), if stderr_open => match line {
                Some(text) => {
                    push_output(output, &text);
                    let _ = tx.unbounded_send(RunEvent::Line { stream: OutputStream::Stderr, text });
                }
                None => stderr_open = false,
            },
            status = child.wait(), if !stdout_open && !stderr_open => {
                return match status {
                    Ok(status) => Status::Completed(status.code().unwrap_or(-1)),
                    Err(error) => Status::Failed(error.to_string()),
                };
            }
            changed = cancel.changed(), if cancel_alive => match changed {
                Ok(()) if *cancel.borrow() => {
                    let _ = child.kill().await;
                    return Status::Cancelled;
                }
                Ok(()) => {}
                Err(_) => cancel_alive = false,
            },
            _ = &mut timeout => {
                let _ = child.kill().await;
                return Status::TimedOut;
            }
        }
    }
}
