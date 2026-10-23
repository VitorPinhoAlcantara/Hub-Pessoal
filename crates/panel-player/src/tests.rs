use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::platform::FakePlatform;
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

fn player(tools: Tools) -> Player {
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    let (mut player, _) = Player::new(ctx);
    let _ = player.update(Message::ToolsChecked(tools));
    player
}

fn send(player: &mut Player, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = player.update(message);
    }
}

fn missing() -> Tools {
    Tools {
        mpv: None,
        ytdlp: Some("2026.10.01".into()),
        ..Tools::default()
    }
}

#[test]
fn without_mpv_play_shows_error() {
    let mut player = player(missing());
    send(
        &mut player,
        [Message::Play("https://www.youtube.com/watch?v=x".into())],
    );
    assert!(!player.is_active());
    assert!(player.error().unwrap().contains("mpv"));
    assert!(player.expanded());
}

#[test]
fn invalid_link_is_rejected() {
    let mut player = player(missing());
    send(
        &mut player,
        [Message::UrlChanged("youtube".into()), Message::PlayInput],
    );
    assert!(player.error().unwrap().contains("link"));
}

#[test]
fn properties_update_status_and_name_favorite() {
    let mut player = player(missing());
    player.current_url = Some("https://youtu.be/a".into());
    hub_core::player::touch_favorite(
        player.ctx.db().conn(),
        "https://youtu.be/a",
        player.ctx.clock.now(),
    )
    .unwrap();
    send(
        &mut player,
        [
            Message::Mpv(MpvEvent::Property(Property::Title(Some("Lo-fi".into())))),
            Message::Mpv(MpvEvent::Property(Property::Duration(Some(200.0)))),
            Message::Mpv(MpvEvent::Property(Property::Position(Some(20.0)))),
            Message::Mpv(MpvEvent::Property(Property::Paused(true))),
            Message::Mpv(MpvEvent::Property(Property::PlaylistPos(Some(1)))),
        ],
    );
    let status = player.status();
    assert_eq!(status.title.as_deref(), Some("Lo-fi"));
    assert!(status.paused);
    assert_eq!(player.position(), 20.0);
    assert_eq!(player.favorites()[0].label(), "Lo-fi");
}

#[test]
fn commands_wait_for_connection_and_are_flushed() {
    let mut player = player(missing());
    send(&mut player, [Message::TogglePause, Message::VolumeChanged(30.0)]);
    assert_eq!(player.queued, vec![Command::TogglePause]);
    let (sender, mut receiver) = futures::channel::mpsc::unbounded();
    send(&mut player, [Message::Mpv(MpvEvent::Connected(sender))]);
    assert!(player.queued.is_empty());
    assert_eq!(receiver.try_recv().unwrap(), Command::TogglePause);
    send(
        &mut player,
        [
            Message::SeekPreview(42.0),
            Message::SeekCommit,
            Message::PlayIndex(3),
        ],
    );
    assert_eq!(receiver.try_recv().unwrap(), Command::Seek(42.0));
    assert_eq!(receiver.try_recv().unwrap(), Command::PlayIndex(3));
}

#[test]
fn volume_is_kept_and_disconnect_resets() {
    let mut player = player(missing());
    send(&mut player, [Message::VolumeChanged(40.0)]);
    assert_eq!(player.status().volume, 40.0);
    assert!(player.queued.is_empty());
    send(
        &mut player,
        [Message::Mpv(MpvEvent::Disconnected("fim".into()))],
    );
    assert_eq!(player.status().volume, 40.0);
    assert!(!player.is_connected());
}

#[test]
fn system_media_is_used_when_mpv_is_idle() {
    let mut player = player(missing());
    send(
        &mut player,
        [Message::SystemLoaded(Ok(Some(
            hub_core::platform::MediaInfo {
                title: "Música do Spotify".into(),
                artist: Some("Artista".into()),
                playing: true,
            },
        )))],
    );
    assert_eq!(player.system().unwrap().title, "Música do Spotify");
    let _ = player.view();
}

#[test]
fn view_builds_collapsed_and_expanded() {
    let mut player = player(missing());
    let _ = player.view();
    send(&mut player, [Message::ToggleExpanded]);
    let _ = player.view();
    let mut ready = self::player(Tools {
        mpv: Some("mpv 0.40".into()),
        ytdlp: Some("2026.10.01".into()),
        ..Tools::default()
    });
    send(
        &mut ready,
        [
            Message::ToggleExpanded,
            Message::Mpv(MpvEvent::Property(Property::Playlist(vec![
                Entry {
                    title: Some("A".into()),
                    filename: "a".into(),
                },
                Entry {
                    title: None,
                    filename: "b".into(),
                },
            ]))),
        ],
    );
    let _ = ready.view();
}

#[test]
fn system_play_pause_updates_optimistically_and_hides_no_media() {
    let mut player = player(missing());
    send(
        &mut player,
        [Message::SystemLoaded(Ok(Some(
            hub_core::platform::MediaInfo {
                title: "Podcast".into(),
                artist: None,
                playing: true,
            },
        )))],
    );
    send(
        &mut player,
        [Message::SystemCommand(
            hub_core::platform::MediaCommand::PlayPause,
        )],
    );
    assert!(!player.system().unwrap().playing);
    send(
        &mut player,
        [Message::SystemCommandDone(Err(
            "operação não suportada: nenhuma mídia tocando".into(),
        ))],
    );
    assert!(player.notice().is_none());
    send(
        &mut player,
        [Message::SystemCommandDone(Err("falhou".into()))],
    );
    assert_eq!(player.notice(), Some("falhou"));
}

fn watch(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

fn info(ids: &[&str]) -> PlaylistInfo {
    PlaylistInfo {
        title: Some("Lista".into()),
        entries: ids
            .iter()
            .map(|id| PlaylistEntry {
                id: (*id).into(),
                title: Some(id.to_uppercase()),
            })
            .collect(),
    }
}

fn entry(id: &str) -> PlaylistEntry {
    PlaylistEntry {
        id: id.into(),
        title: Some(id.to_uppercase()),
    }
}

fn queue_of(player: &mut Player, ids: &[&str], index: usize) {
    player.queue = ids.iter().map(|id| entry(id)).collect();
    player.index = index;
}

#[test]
fn resolved_playlist_builds_queue_around_current_video() {
    let mut player = player(missing());
    queue_of(&mut player, &["c"], 0);
    player.listing = Some(Listing {
        key: "k".into(),
        video: Some("c".into()),
    });
    send(
        &mut player,
        [Message::PlaylistResolved(
            "k".into(),
            Some(info(&["a", "b", "c", "d"])),
        )],
    );
    assert_eq!(player.queue.len(), 4);
    assert_eq!(player.index, 2);
    assert_eq!(
        player.queued,
        vec![Command::Append {
            url: watch("d"),
            title: Some("D".into())
        }]
    );
    let items = player.queue_items();
    assert_eq!(items[2], ("C".to_string(), true));
    assert!(!player.loading_playlist());
}

#[test]
fn playlist_only_link_loads_first_item_when_list_arrives() {
    let mut player = player(missing());
    player.listing = Some(Listing {
        key: "k".into(),
        video: None,
    });
    player.waiting_list = true;
    assert!(player.loading_playlist());
    send(
        &mut player,
        [Message::PlaylistResolved(
            "k".into(),
            Some(info(&["a", "b"])),
        )],
    );
    assert_eq!(
        player.queued,
        vec![
            Command::Load {
                url: watch("a"),
                title: Some("A".into())
            },
            Command::Append {
                url: watch("b"),
                title: Some("B".into())
            },
        ]
    );
    assert!(!player.loading_playlist());
}

#[test]
fn failed_list_without_first_item_shows_error_and_stale_key_is_ignored() {
    let mut player = player(missing());
    player.listing = Some(Listing {
        key: "k".into(),
        video: None,
    });
    send(
        &mut player,
        [Message::PlaylistResolved("outra".into(), None)],
    );
    assert!(player.error().is_none());
    send(&mut player, [Message::PlaylistResolved("k".into(), None)]);
    assert!(player.error().is_some());
}

#[test]
fn command_failure_with_title_is_retried_without_title() {
    let mut player = player(missing());
    let titled = Command::Load {
        url: watch("a"),
        title: Some("A".into()),
    };
    send(
        &mut player,
        [Message::Mpv(MpvEvent::Failed(titled, "invalid".into()))],
    );
    assert!(!player.titles_ok);
    assert_eq!(
        player.queued,
        vec![Command::Load {
            url: watch("a"),
            title: None
        }]
    );
}

#[test]
fn finishing_a_track_moves_to_preloaded_next_and_prefetches_again() {
    let mut player = player(missing());
    queue_of(&mut player, &["a", "b", "c"], 0);
    player.preloaded = Some("b".into());
    send(
        &mut player,
        [Message::Mpv(MpvEvent::Property(Property::PlaylistPos(Some(1))))],
    );
    assert_eq!(player.index, 1);
    assert_eq!(
        player.queued,
        vec![
            Command::PlaylistRemove(0),
            Command::Append {
                url: watch("c"),
                title: Some("C".into())
            },
        ]
    );
    player.queued.clear();
    send(
        &mut player,
        [Message::Mpv(MpvEvent::Property(Property::PlaylistPos(Some(0))))],
    );
    assert_eq!(player.index, 1);
    assert!(player.queued.is_empty());
}

#[test]
fn direct_url_is_appended_only_while_still_next() {
    let mut player = player(missing());
    queue_of(&mut player, &["a", "b", "c"], 0);
    send(
        &mut player,
        [Message::DirectResolved("c".into(), Some("https://cdn/c".into()))],
    );
    assert!(player.queued.is_empty());
    send(
        &mut player,
        [Message::DirectResolved("b".into(), Some("https://cdn/b".into()))],
    );
    assert_eq!(
        player.queued,
        vec![Command::Append {
            url: "https://cdn/b".into(),
            title: Some("B".into())
        }]
    );
    player.queued.clear();
    send(
        &mut player,
        [Message::DirectResolved("b".into(), Some("https://cdn/b2".into()))],
    );
    assert!(player.queued.is_empty());
}

#[test]
fn skip_and_queue_click_load_tracks_from_own_queue() {
    let mut player = player(missing());
    queue_of(&mut player, &["a", "b", "c"], 0);
    send(&mut player, [Message::Next]);
    assert_eq!(player.index, 1);
    assert_eq!(
        player.queued[0],
        Command::Load {
            url: watch("b"),
            title: Some("B".into())
        }
    );
    player.queued.clear();
    send(&mut player, [Message::PlayIndex(0)]);
    assert_eq!(player.index, 0);
    assert_eq!(
        player.queued[0],
        Command::Load {
            url: watch("a"),
            title: Some("A".into())
        }
    );
    player.queued.clear();
    send(&mut player, [Message::Previous]);
    assert!(player.queued.is_empty());
}

#[test]
fn skip_moves_between_favorites_when_queue_has_no_neighbor() {
    let mut player = player(missing());
    player.favorite_order = vec![
        "https://youtu.be/a".into(),
        "https://youtu.be/b".into(),
        "https://youtu.be/c".into(),
    ];
    player.current_url = Some("https://youtu.be/b".into());
    assert_eq!(player.favorite_neighbor(1).as_deref(), Some("https://youtu.be/c"));
    assert_eq!(player.favorite_neighbor(-1).as_deref(), Some("https://youtu.be/a"));
    player.current_url = Some("https://youtu.be/c".into());
    assert_eq!(player.favorite_neighbor(1), None);
}

#[test]
fn placeholder_titles_do_not_name_favorites() {
    assert!(hub_core::player::is_placeholder_title("watch?v=S02YkyF9u1o"));
    assert!(hub_core::player::is_placeholder_title("https://www.youtube.com/watch?v=a"));
    assert!(!hub_core::player::is_placeholder_title("Flegrea e Hornacis"));
}
