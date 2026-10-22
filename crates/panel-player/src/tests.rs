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
    send(&mut player, [Message::TogglePause, Message::Next]);
    assert_eq!(player.queued, vec![Command::TogglePause, Command::Next]);
    let (sender, mut receiver) = futures::channel::mpsc::unbounded();
    send(&mut player, [Message::Mpv(MpvEvent::Connected(sender))]);
    assert!(player.queued.is_empty());
    assert_eq!(receiver.try_recv().unwrap(), Command::TogglePause);
    assert_eq!(receiver.try_recv().unwrap(), Command::Next);
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
