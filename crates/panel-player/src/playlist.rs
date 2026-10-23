use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hub_core::platform;

pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(20);
pub const DIRECT_TIMEOUT: Duration = Duration::from_secs(30);
pub const DIRECT_TTL: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkIds {
    pub video: Option<String>,
    pub list: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    pub id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistInfo {
    pub title: Option<String>,
    pub entries: Vec<PlaylistEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadPlan {
    pub first: Option<String>,
    pub lookup: Option<String>,
    pub start_video: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Queue {
    pub current: Option<PlaylistEntry>,
    pub before: Vec<PlaylistEntry>,
    pub after: Vec<PlaylistEntry>,
}

fn query_value(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or(query);
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, value)| *name == key && !value.is_empty())
        .map(|(_, value)| value.to_string())
}

pub fn link_ids(url: &str) -> LinkIds {
    let url = url.trim();
    let mut video = query_value(url, "v");
    if video.is_none()
        && let Some(rest) = url
            .strip_prefix("https://youtu.be/")
            .or_else(|| url.strip_prefix("http://youtu.be/"))
    {
        video = rest
            .split(['?', '&', '#', '/'])
            .next()
            .filter(|id| !id.is_empty())
            .map(str::to_string);
    }
    LinkIds {
        video,
        list: query_value(url, "list"),
    }
}

pub fn playlist_url(list: &str) -> String {
    format!("https://www.youtube.com/playlist?list={list}")
}

pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

pub fn video_id_of(filename: &str) -> Option<String> {
    let rest = filename
        .strip_prefix("https://www.youtube.com/watch?")
        .or_else(|| filename.strip_prefix("https://youtube.com/watch?"))?;
    query_value(&format!("?{rest}"), "v")
}

pub fn is_mix(list: &str) -> bool {
    list.starts_with("RD")
}

pub fn plan(url: &str) -> LoadPlan {
    let url = url.trim();
    let ids = link_ids(url);
    match ids.list {
        Some(list) if !is_mix(&list) => LoadPlan {
            first: ids.video.as_deref().map(watch_url),
            lookup: Some(playlist_url(&list)),
            start_video: ids.video,
        },
        _ => LoadPlan {
            first: Some(url.to_string()),
            lookup: None,
            start_video: None,
        },
    }
}

fn clean(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty() && text != "NA").then(|| text.to_string())
}

pub fn parse_info(output: &str) -> Option<PlaylistInfo> {
    let mut title = None;
    let mut entries = Vec::new();
    for line in output.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(list_title), Some(id), Some(entry_title)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() || id == "NA" {
            continue;
        }
        if title.is_none() {
            title = clean(list_title);
        }
        entries.push(PlaylistEntry {
            id: id.to_string(),
            title: clean(entry_title),
        });
    }
    if entries.is_empty() {
        return None;
    }
    Some(PlaylistInfo { title, entries })
}

pub fn split_queue(entries: &[PlaylistEntry], video: Option<&str>) -> Queue {
    match video {
        Some(video) => match entries.iter().position(|entry| entry.id == video) {
            Some(index) => Queue {
                current: Some(entries[index].clone()),
                before: entries[..index].to_vec(),
                after: entries[index + 1..].to_vec(),
            },
            None => Queue {
                current: None,
                before: Vec::new(),
                after: entries.to_vec(),
            },
        },
        None => Queue {
            current: entries.first().cloned(),
            before: Vec::new(),
            after: entries.iter().skip(1).cloned().collect(),
        },
    }
}

#[derive(Debug, Default)]
pub struct DirectCache {
    entries: HashMap<String, (String, Instant)>,
}

impl DirectCache {
    pub fn insert(&mut self, id: &str, url: String, now: Instant) {
        self.entries.insert(id.to_string(), (url, now));
    }

    pub fn get(&self, id: &str, now: Instant) -> Option<&str> {
        let (url, stored) = self.entries.get(id)?;
        (now.saturating_duration_since(*stored) < DIRECT_TTL).then_some(url.as_str())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn run_ytdlp(ytdlp: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut command = std::process::Command::new(ytdlp);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    platform::hide_console_std(&mut command);
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    match rx.recv_timeout(timeout) {
        Ok(bytes) => {
            let _ = child.wait();
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

pub fn cookie_args(cookies: Option<&str>) -> Vec<String> {
    match cookies {
        Some(source) => vec!["--cookies-from-browser".to_string(), source.to_string()],
        None => Vec::new(),
    }
}

fn run_with_cookies(
    ytdlp: &Path,
    args: &[&str],
    cookies: Option<&str>,
    timeout: Duration,
) -> Option<String> {
    let mut all = cookie_args(cookies);
    all.extend(args.iter().map(|arg| arg.to_string()));
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    run_ytdlp(ytdlp, &refs, timeout)
}

pub fn fetch_info(ytdlp: &Path, url: &str, cookies: Option<&str>) -> Option<PlaylistInfo> {
    let args = [
        "--flat-playlist",
        "--no-warnings",
        "--encoding",
        "utf-8",
        "--print",
        "%(playlist_title)s\t%(id)s\t%(title)s",
        url,
    ];
    let parsed = run_with_cookies(ytdlp, &args, cookies, LOOKUP_TIMEOUT)
        .and_then(|output| parse_info(&output));
    if parsed.is_some() || cookies.is_none() {
        return parsed;
    }
    run_with_cookies(ytdlp, &args, None, LOOKUP_TIMEOUT).and_then(|output| parse_info(&output))
}

pub fn parse_direct(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("http://") || line.starts_with("https://"))
        .map(str::to_string)
}

pub fn fetch_direct(ytdlp: &Path, id: &str) -> Option<String> {
    let url = watch_url(id);
    let args = [
        "-f",
        "bestaudio/best",
        "-g",
        "--no-playlist",
        "--no-warnings",
        "--encoding",
        "utf-8",
        url.as_str(),
    ];
    run_ytdlp(ytdlp, &args, DIRECT_TIMEOUT).and_then(|output| parse_direct(&output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> PlaylistEntry {
        PlaylistEntry {
            id: id.into(),
            title: Some(id.to_uppercase()),
        }
    }

    fn ids(entries: &[PlaylistEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn ids_are_extracted() {
        let ids = link_ids("https://www.youtube.com/watch?v=abc123&list=PLxyz&index=4");
        assert_eq!(ids.video.as_deref(), Some("abc123"));
        assert_eq!(ids.list.as_deref(), Some("PLxyz"));
        let ids = link_ids("https://www.youtube.com/playlist?list=PLxyz");
        assert_eq!(ids.video, None);
        assert_eq!(ids.list.as_deref(), Some("PLxyz"));
        let ids = link_ids("https://youtu.be/abc123?list=PLq");
        assert_eq!(ids.video.as_deref(), Some("abc123"));
        assert_eq!(ids.list.as_deref(), Some("PLq"));
        assert_eq!(link_ids("https://youtu.be/abc123").list, None);
        assert_eq!(link_ids("https://x.com/a"), LinkIds::default());
    }

    #[test]
    fn plans_follow_the_link_kind() {
        let single = plan("https://www.youtube.com/watch?v=abc");
        assert_eq!(
            single.first.as_deref(),
            Some("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(single.lookup, None);
        let listed = plan("https://www.youtube.com/watch?v=abc&list=PLxyz&index=2");
        assert_eq!(
            listed.first.as_deref(),
            Some("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(
            listed.lookup.as_deref(),
            Some("https://www.youtube.com/playlist?list=PLxyz")
        );
        assert_eq!(listed.start_video.as_deref(), Some("abc"));
        let whole = plan("https://www.youtube.com/playlist?list=PLxyz");
        assert_eq!(whole.first, None);
        assert_eq!(whole.start_video, None);
        assert!(whole.lookup.is_some());
        let mix = plan("https://www.youtube.com/watch?v=abc&list=RDabc");
        assert_eq!(
            mix.first.as_deref(),
            Some("https://www.youtube.com/watch?v=abc&list=RDabc")
        );
        assert_eq!(mix.lookup, None);
    }

    #[test]
    fn info_parses_title_entries_and_skips_invalid_lines() {
        let output = "Meus Lofi\ta1\tPrimeira\nMeus Lofi\tb2\tNA\nlixo sem tab\nMeus Lofi\t\tsem id\nMeus Lofi\tc3\tCom\ttab\n";
        let info = parse_info(output).unwrap();
        assert_eq!(info.title.as_deref(), Some("Meus Lofi"));
        assert_eq!(info.entries.len(), 3);
        assert_eq!(info.entries[0].title.as_deref(), Some("Primeira"));
        assert_eq!(info.entries[1].title, None);
        assert_eq!(info.entries[2].title.as_deref(), Some("Com\ttab"));
        let late = parse_info("NA\ta1\tA\nLista\tb2\tB\n").unwrap();
        assert_eq!(late.title.as_deref(), Some("Lista"));
        assert!(parse_info("").is_none());
        assert!(parse_info("so\tdois\n").is_none());
    }

    #[test]
    fn queue_splits_around_current_video() {
        let entries: Vec<_> = ["a", "b", "c", "d"].into_iter().map(entry).collect();
        let queue = split_queue(&entries, Some("c"));
        assert_eq!(queue.current.unwrap().id, "c");
        assert_eq!(ids(&queue.before), ["a", "b"]);
        assert_eq!(ids(&queue.after), ["d"]);
        let first = split_queue(&entries, Some("a"));
        assert!(first.before.is_empty());
        assert_eq!(first.after.len(), 3);
        let missing = split_queue(&entries, Some("zz"));
        assert!(missing.current.is_none());
        assert!(missing.before.is_empty());
        assert_eq!(missing.after.len(), 4);
        let whole = split_queue(&entries, None);
        assert_eq!(whole.current.unwrap().id, "a");
        assert!(whole.before.is_empty());
        assert_eq!(ids(&whole.after), ["b", "c", "d"]);
    }

    #[test]
    fn direct_urls_expire_after_two_hours() {
        let start = Instant::now();
        let mut cache = DirectCache::default();
        cache.insert("a", "https://cdn/a".into(), start);
        assert_eq!(cache.get("a", start), Some("https://cdn/a"));
        assert_eq!(
            cache.get("a", start + DIRECT_TTL - Duration::from_secs(1)),
            Some("https://cdn/a")
        );
        assert_eq!(cache.get("a", start + DIRECT_TTL), None);
        assert_eq!(cache.get("b", start), None);
    }

    #[test]
    fn direct_output_and_watch_ids() {
        assert_eq!(
            parse_direct("WARNING x\nhttps://cdn/audio?x=1\n").as_deref(),
            Some("https://cdn/audio?x=1")
        );
        assert_eq!(parse_direct("erro\n"), None);
        assert_eq!(
            video_id_of("https://www.youtube.com/watch?v=abc123").as_deref(),
            Some("abc123")
        );
        assert_eq!(video_id_of("https://cdn/audio"), None);
    }
}
