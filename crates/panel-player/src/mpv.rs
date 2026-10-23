use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::Duration;

use futures::channel::mpsc;
use futures::{FutureExt, Stream, StreamExt};
use hub_core::platform::{self, ipc};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub const OBSERVED: [&str; 8] = [
    "media-title",
    "time-pos",
    "duration",
    "pause",
    "playlist-pos",
    "playlist",
    "volume",
    "idle-active",
];

const CONNECT_ATTEMPTS: usize = 50;
const CONNECT_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Load {
        url: String,
        title: Option<String>,
    },
    Append {
        url: String,
        title: Option<String>,
    },
    InsertAt {
        url: String,
        index: usize,
        title: Option<String>,
    },
    PlaylistRemove(usize),
    PlaylistMove {
        from: usize,
        to: usize,
    },
    TogglePause,
    Next,
    Previous,
    SetVolume(f64),
    Seek(f64),
    PlayIndex(usize),
    Quit,
}

pub fn file_options(title: &str) -> String {
    format!("force-media-title=%{}%{}", title.len(), title)
}

fn load_arguments(url: &str, flag: &str, index: Option<usize>, title: Option<&str>) -> Value {
    match (title, index) {
        (Some(title), index) => {
            let index = index.map_or(-1, |index| index as i64);
            json!(["loadfile", url, flag, index, file_options(title)])
        }
        (None, Some(index)) => json!(["loadfile", url, flag, index]),
        (None, None) => json!(["loadfile", url, flag]),
    }
}

impl Command {
    pub fn without_title(&self) -> Option<Command> {
        match self {
            Command::Load {
                url,
                title: Some(_),
            } => Some(Command::Load {
                url: url.clone(),
                title: None,
            }),
            Command::Append {
                url,
                title: Some(_),
            } => Some(Command::Append {
                url: url.clone(),
                title: None,
            }),
            Command::InsertAt {
                url,
                index,
                title: Some(_),
            } => Some(Command::InsertAt {
                url: url.clone(),
                index: *index,
                title: None,
            }),
            _ => None,
        }
    }

    pub fn arguments(&self) -> Value {
        match self {
            Command::Load { url, title } => load_arguments(url, "replace", None, title.as_deref()),
            Command::Append { url, title } => load_arguments(url, "append", None, title.as_deref()),
            Command::InsertAt { url, index, title } => {
                load_arguments(url, "insert-at", Some(*index), title.as_deref())
            }
            Command::PlaylistRemove(index) => json!(["playlist-remove", index]),
            Command::PlaylistMove { from, to } => json!(["playlist-move", from, to]),
            Command::TogglePause => json!(["cycle", "pause"]),
            Command::Next => json!(["playlist-next", "weak"]),
            Command::Previous => json!(["playlist-prev", "weak"]),
            Command::SetVolume(volume) => json!(["set_property", "volume", volume]),
            Command::Seek(seconds) => json!(["seek", seconds, "absolute"]),
            Command::PlayIndex(index) => json!(["playlist-play-index", index]),
            Command::Quit => json!(["quit"]),
        }
    }
}

pub fn encode(command: &Command, request_id: u64) -> String {
    format!(
        "{}\n",
        json!({ "command": command.arguments(), "request_id": request_id })
    )
}

pub fn observe(id: usize, property: &str) -> String {
    format!(
        "{}\n",
        json!({ "command": ["observe_property", id, property] })
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub title: Option<String>,
    pub filename: String,
}

impl Entry {
    pub fn label(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.filename)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Property {
    Title(Option<String>),
    Position(Option<f64>),
    Duration(Option<f64>),
    Paused(bool),
    PlaylistPos(Option<usize>),
    Playlist(Vec<Entry>),
    Volume(f64),
    Idle(bool),
}

pub fn parse(line: &str) -> Option<Property> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("event").and_then(Value::as_str) != Some("property-change") {
        return None;
    }
    let data = value.get("data").unwrap_or(&Value::Null);
    let text = || data.as_str().map(str::to_string);
    Some(match value.get("name").and_then(Value::as_str)? {
        "media-title" => Property::Title(text()),
        "time-pos" => Property::Position(data.as_f64()),
        "duration" => Property::Duration(data.as_f64()),
        "pause" => Property::Paused(data.as_bool().unwrap_or(false)),
        "playlist-pos" => Property::PlaylistPos(
            data.as_i64()
                .filter(|p| *p >= 0)
                .and_then(|p| usize::try_from(p).ok()),
        ),
        "playlist" => Property::Playlist(
            data.as_array()
                .into_iter()
                .flatten()
                .map(|item| Entry {
                    title: item
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    filename: item
                        .get("filename")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect(),
        ),
        "volume" => Property::Volume(data.as_f64().unwrap_or(100.0)),
        "idle-active" => Property::Idle(data.as_bool().unwrap_or(false)),
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub enum MpvEvent {
    Connected(mpsc::UnboundedSender<Command>),
    Property(Property),
    Failed(Command, String),
    Disconnected(String),
}

pub fn parse_response(line: &str) -> Option<(u64, Option<String>)> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let id = value.get("request_id")?.as_u64()?;
    let error = value
        .get("error")
        .and_then(Value::as_str)
        .filter(|error| *error != "success")
        .map(str::to_string);
    Some((id, error))
}

pub fn connect(path: String) -> impl Stream<Item = MpvEvent> + Send + 'static {
    let (out, events) = mpsc::unbounded();
    let driver = drive(path, out)
        .into_stream()
        .filter_map(|_| async { None });
    futures::stream::select(driver, events)
}

async fn drive(path: String, out: mpsc::UnboundedSender<MpvEvent>) {
    let mut connection = None;
    for _ in 0..CONNECT_ATTEMPTS {
        match ipc::connect(&path).await {
            Ok(stream) => {
                connection = Some(stream);
                break;
            }
            Err(_) => tokio::time::sleep(CONNECT_DELAY).await,
        }
    }
    let Some(stream) = connection else {
        let _ = out.unbounded_send(MpvEvent::Disconnected(
            "Não foi possível conectar ao mpv".to_string(),
        ));
        return;
    };
    let (reader, mut writer) = tokio::io::split(stream);
    for (index, property) in OBSERVED.iter().enumerate() {
        if writer
            .write_all(observe(index + 1, property).as_bytes())
            .await
            .is_err()
        {
            let _ = out.unbounded_send(MpvEvent::Disconnected("mpv recusou a conexão".to_string()));
            return;
        }
    }
    let (commands_tx, mut commands) = mpsc::unbounded::<Command>();
    let _ = out.unbounded_send(MpvEvent::Connected(commands_tx));
    let mut lines = BufReader::new(reader).lines();
    let mut request_id = 100;
    let mut inflight: HashMap<u64, Command> = HashMap::new();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Some(property) = parse(&line) {
                        let _ = out.unbounded_send(MpvEvent::Property(property));
                    } else if let Some((id, error)) = parse_response(&line)
                        && let Some(command) = inflight.remove(&id)
                        && let Some(error) = error
                    {
                        let _ = out.unbounded_send(MpvEvent::Failed(command, error));
                    }
                }
                _ => {
                    let _ = out.unbounded_send(MpvEvent::Disconnected("O mpv foi encerrado".to_string()));
                    return;
                }
            },
            command = commands.next() => match command {
                Some(command) => {
                    request_id += 1;
                    inflight.insert(request_id, command.clone());
                    if writer.write_all(encode(&command, request_id).as_bytes()).await.is_err() {
                        let _ = out.unbounded_send(MpvEvent::Disconnected("O mpv parou de responder".to_string()));
                        return;
                    }
                }
                None => return,
            },
        }
    }
}

pub fn spawn_args(ytdlp: Option<&Path>, ipc_path: &str, volume: f64) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(ytdlp) = ytdlp {
        args.push(format!(
            "--script-opts=ytdl_hook-ytdl_path={}",
            ytdlp.display()
        ));
    }
    args.extend([
        "--idle=yes".to_string(),
        "--no-video".to_string(),
        "--no-terminal".to_string(),
        "--force-window=no".to_string(),
        "--ytdl-format=bestaudio/best".to_string(),
        format!("--volume={volume}"),
        format!("--input-ipc-server={ipc_path}"),
    ]);
    args
}

pub struct MpvProcess {
    child: Child,
}

impl MpvProcess {
    pub fn spawn(
        program: &Path,
        ytdlp: Option<&Path>,
        ipc_path: &str,
        volume: f64,
    ) -> std::io::Result<Self> {
        let mut command = std::process::Command::new(program);
        command.args(spawn_args(ytdlp, ipc_path, volume));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        platform::hide_console_std(&mut command);
        Ok(Self {
            child: command.spawn()?,
        })
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for MpvProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools {
    pub mpv: Option<String>,
    pub ytdlp: Option<String>,
    pub mpv_path: Option<PathBuf>,
    pub ytdlp_path: Option<PathBuf>,
}

impl Tools {
    pub fn ready(&self) -> bool {
        self.mpv.is_some() && self.ytdlp.is_some()
    }
}

pub fn version(program: impl AsRef<OsStr>, arg: &str) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.arg(arg).stdin(Stdio::null());
    platform::hide_console_std(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
}

pub fn check_tools() -> Tools {
    let mpv_path = platform::find_program("mpv");
    let ytdlp_path = platform::find_program("yt-dlp");
    Tools {
        mpv: mpv_path
            .as_ref()
            .and_then(|path| version(path, "--version")),
        ytdlp: ytdlp_path
            .as_ref()
            .and_then(|path| version(path, "--version")),
        mpv_path,
        ytdlp_path,
    }
}

pub fn update_ytdlp() -> String {
    let program = platform::find_program("yt-dlp").unwrap_or_else(|| PathBuf::from("yt-dlp"));
    let mut command = std::process::Command::new(program);
    command.arg("-U").stdin(Stdio::null());
    platform::hide_console_std(&mut command);
    match command.output() {
        Ok(output) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            text.lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("Pronto")
                .trim()
                .to_string()
        }
        Err(error) => format!("Não foi possível rodar o yt-dlp: {error}"),
    }
}

pub fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (hours, minutes, secs) = (total / 3600, (total / 60) % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_json_lines() {
        let line = encode(
            &Command::Load {
                url: "https://x".into(),
                title: None,
            },
            7,
        );
        assert!(line.ends_with('\n'));
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(
            value["command"],
            json!(["loadfile", "https://x", "replace"])
        );
        assert_eq!(value["request_id"], 7);
        assert_eq!(
            Command::Seek(12.5).arguments(),
            json!(["seek", 12.5, "absolute"])
        );
        assert_eq!(
            Command::PlayIndex(2).arguments(),
            json!(["playlist-play-index", 2])
        );
        let observed: Value = serde_json::from_str(observe(3, "pause").trim()).unwrap();
        assert_eq!(observed["command"], json!(["observe_property", 3, "pause"]));
    }

    #[test]
    fn property_changes_are_parsed() {
        let parse_line = |text: &str| parse(text).unwrap();
        assert_eq!(
            parse_line(r#"{"event":"property-change","id":1,"name":"media-title","data":"Lo-fi"}"#),
            Property::Title(Some("Lo-fi".into()))
        );
        assert_eq!(
            parse_line(r#"{"event":"property-change","id":2,"name":"time-pos","data":12.5}"#),
            Property::Position(Some(12.5))
        );
        assert_eq!(
            parse_line(r#"{"event":"property-change","id":5,"name":"playlist-pos","data":-1}"#),
            Property::PlaylistPos(None)
        );
        assert_eq!(
            parse_line(
                r#"{"event":"property-change","id":6,"name":"playlist","data":[{"filename":"a","title":"A"},{"filename":"b"}]}"#
            ),
            Property::Playlist(vec![
                Entry {
                    title: Some("A".into()),
                    filename: "a".into()
                },
                Entry {
                    title: None,
                    filename: "b".into()
                },
            ])
        );
        assert!(parse(r#"{"request_id":1,"error":"success"}"#).is_none());
        assert!(parse("lixo").is_none());
    }

    #[test]
    fn spawn_args_set_idle_audio_only_and_no_playlist_option() {
        let args = spawn_args(Some(Path::new("yt-dlp.exe")), "pipe", 70.0);
        assert!(args.contains(&"--idle=yes".to_string()));
        assert!(!args.iter().any(|a| a.contains("yes-playlist")));
        assert!(!args.iter().any(|a| a.starts_with("--keep-open")));
        assert!(args.contains(&"--script-opts=ytdl_hook-ytdl_path=yt-dlp.exe".to_string()));
        assert!(args.contains(&"--input-ipc-server=pipe".to_string()));
        assert_eq!(spawn_args(None, "p", 5.0).len(), args.len() - 1);
    }

    #[test]
    fn loadfile_commands_cover_append_insert_and_titles() {
        let url = "https://www.youtube.com/watch?v=a";
        let load = |title: Option<&str>| Command::Load {
            url: url.into(),
            title: title.map(str::to_string),
        };
        assert_eq!(load(None).arguments(), json!(["loadfile", url, "replace"]));
        assert_eq!(
            load(Some("Oi")).arguments(),
            json!(["loadfile", url, "replace", -1, "force-media-title=%2%Oi"])
        );
        let append = Command::Append {
            url: url.into(),
            title: None,
        };
        assert_eq!(append.arguments(), json!(["loadfile", url, "append"]));
        let append_titled = Command::Append {
            url: url.into(),
            title: Some("A, B".into()),
        };
        assert_eq!(
            append_titled.arguments(),
            json!(["loadfile", url, "append", -1, "force-media-title=%4%A, B"])
        );
        let insert = Command::InsertAt {
            url: url.into(),
            index: 2,
            title: None,
        };
        assert_eq!(insert.arguments(), json!(["loadfile", url, "insert-at", 2]));
        let insert_titled = Command::InsertAt {
            url: url.into(),
            index: 0,
            title: Some("Música".into()),
        };
        assert_eq!(
            insert_titled.arguments(),
            json!([
                "loadfile",
                url,
                "insert-at",
                0,
                "force-media-title=%7%Música"
            ])
        );
        assert_eq!(
            insert_titled.without_title(),
            Some(Command::InsertAt {
                url: url.into(),
                index: 0,
                title: None
            })
        );
        assert_eq!(insert.without_title(), None);
        assert_eq!(
            Command::PlaylistRemove(3).arguments(),
            json!(["playlist-remove", 3])
        );
        assert_eq!(
            Command::PlaylistMove { from: 5, to: 1 }.arguments(),
            json!(["playlist-move", 5, 1])
        );
    }

    #[test]
    fn responses_report_errors_only() {
        assert_eq!(
            parse_response(r#"{"request_id":101,"error":"success"}"#),
            Some((101, None))
        );
        assert_eq!(
            parse_response(r#"{"request_id":102,"error":"invalid parameter"}"#),
            Some((102, Some("invalid parameter".into())))
        );
        assert_eq!(parse_response(r#"{"event":"file-loaded"}"#), None);
    }

    #[test]
    fn times_are_formatted() {
        assert_eq!(format_time(5.0), "0:05");
        assert_eq!(format_time(125.9), "2:05");
        assert_eq!(format_time(3725.0), "1:02:05");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_talks_to_fake_mpv_over_ipc() {
        let path = ipc::ipc_path(&format!("hub-mpv-teste-{}", std::process::id()));
        let mut listener = ipc::IpcListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let stream = listener.accept().await.unwrap();
            let (reader, mut writer) = tokio::io::split(stream);
            let mut lines = BufReader::new(reader).lines();
            let mut received = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                let value: Value = serde_json::from_str(&line).unwrap();
                received.push(value["command"].clone());
                if value["command"][0] == "loadfile" {
                    writer
                        .write_all(
                            b"{\"event\":\"property-change\",\"id\":1,\"name\":\"media-title\",\"data\":\"Teste\"}\n",
                        )
                        .await
                        .unwrap();
                }
                if value["command"][0] == "quit" {
                    break;
                }
            }
            received
        });
        let mut events = Box::pin(connect(path));
        let first = tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .expect("travou em conectar");
        let Some(MpvEvent::Connected(sender)) = first else {
            panic!("esperava conexão")
        };
        sender
            .unbounded_send(Command::Load {
                url: "https://youtu.be/x".into(),
                title: None,
            })
            .unwrap();
        let second = tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .expect("travou em propriedade");
        let Some(MpvEvent::Property(property)) = second else {
            panic!("esperava propriedade")
        };
        assert_eq!(property, Property::Title(Some("Teste".into())));
        sender.unbounded_send(Command::Quit).unwrap();
        let rest = tokio::spawn(async move {
            let mut last = None;
            while let Some(event) = events.next().await {
                last = Some(event);
            }
            last
        });
        let received = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("travou em servidor")
            .unwrap();
        assert_eq!(received.len(), OBSERVED.len() + 2);
        assert_eq!(received[0], json!(["observe_property", 1, "media-title"]));
        let last = tokio::time::timeout(Duration::from_secs(5), rest)
            .await
            .expect("travou em desconectar")
            .unwrap();
        assert!(matches!(last, Some(MpvEvent::Disconnected(_))));
    }

    #[test]
    fn missing_program_has_no_version() {
        assert!(version("programa-que-nao-existe-hub", "--version").is_none());
    }
}
