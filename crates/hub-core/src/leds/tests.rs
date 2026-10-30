use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

type Log = Arc<Mutex<Vec<(u32, u32, Vec<u8>)>>>;

fn mode_bytes(name: &str, flags: u32, version: u32) -> Vec<u8> {
    let mode = Mode {
        name: name.to_string(),
        value: 7,
        flags,
        speed_min: 1,
        speed_max: 5,
        brightness_min: 0,
        brightness_max: 100,
        colors_min: 1,
        colors_max: 2,
        speed: 3,
        brightness: 50,
        direction: 0,
        color_mode: 2,
        colors: vec![[1, 2, 3]],
    };
    encode_mode(&mode, version)
}

fn controller_bytes(version: u32) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, 5);
    put_string(&mut body, "Teclado Teste");
    if version >= 1 {
        put_string(&mut body, "Fabricante");
    }
    put_string(&mut body, "descricao");
    put_string(&mut body, "1.0");
    put_string(&mut body, "serial");
    put_string(&mut body, "usb");
    body.extend_from_slice(&2u16.to_le_bytes());
    put_u32(&mut body, 0);
    body.extend(mode_bytes("Direct", FLAG_PER_LED_COLOR, version));
    body.extend(mode_bytes(
        "Breathing",
        FLAG_SPEED | FLAG_MODE_COLOR,
        version,
    ));
    body.extend_from_slice(&1u16.to_le_bytes());
    put_string(&mut body, "Zona");
    for value in [0, 1, 3, 3] {
        put_u32(&mut body, value);
    }
    body.extend_from_slice(&0u16.to_le_bytes());
    if version >= 4 {
        body.extend_from_slice(&1u16.to_le_bytes());
        put_string(&mut body, "Segmento");
        for value in [0, 0, 3] {
            put_u32(&mut body, value);
        }
    }
    body.extend_from_slice(&3u16.to_le_bytes());
    for name in ["a", "b", "c"] {
        put_string(&mut body, name);
        if version < 6 {
            put_u32(&mut body, 0);
        }
    }
    body.extend_from_slice(&3u16.to_le_bytes());
    for _ in 0..3 {
        put_color(&mut body, [9, 9, 9]);
    }
    let mut data = Vec::new();
    put_u32(&mut data, (body.len() + 4) as u32);
    data.extend(body);
    data
}

fn profiles_bytes() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&2u16.to_le_bytes());
    put_string(&mut body, "Jogo");
    put_string(&mut body, "Normal");
    let mut data = Vec::new();
    put_u32(&mut data, (body.len() + 4) as u32);
    data.extend(body);
    data
}

async fn serve(mut stream: TcpStream, log: Log) {
    loop {
        let mut header = [0u8; 16];
        if stream.read_exact(&mut header).await.is_err() {
            return;
        }
        let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
        let (device, id, size) = (word(4), word(8), word(12) as usize);
        let mut data = vec![0u8; size];
        if stream.read_exact(&mut data).await.is_err() {
            return;
        }
        log.lock().unwrap().push((device, id, data));
        let reply = match id {
            40 => Some(6u32.to_le_bytes().to_vec()),
            0 => {
                let noise = encode_packet(0, 100, &[]);
                stream.write_all(&noise).await.unwrap();
                Some(1u32.to_le_bytes().to_vec())
            }
            1 => Some(controller_bytes(4)),
            150 => Some(profiles_bytes()),
            _ => None,
        };
        if let Some(reply) = reply {
            stream
                .write_all(&encode_packet(device, id, &reply))
                .await
                .unwrap();
        }
    }
}

async fn fake_server() -> (u16, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log: Log = Arc::default();
    let shared = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(serve(stream, shared.clone()));
        }
    });
    (port, log)
}

fn sent(log: &Log, id: u32) -> Vec<Vec<u8>> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|(_, packet, _)| *packet == id)
        .map(|(_, _, data)| data.clone())
        .collect()
}

#[test]
fn packet_header_layout() {
    let packet = encode_packet(2, 1050, &[7, 8]);
    assert_eq!(&packet[..4], b"ORGB");
    assert_eq!(&packet[4..8], &2u32.to_le_bytes());
    assert_eq!(&packet[8..12], &1050u32.to_le_bytes());
    assert_eq!(&packet[12..16], &2u32.to_le_bytes());
    assert_eq!(&packet[16..], &[7, 8]);
}

#[test]
fn decodes_controller_for_each_version() {
    for version in [1, 3, 4] {
        let device = decode_device(2, &controller_bytes(version), version).unwrap();
        assert_eq!(device.index, 2);
        assert_eq!(device.kind, DeviceKind::Keyboard);
        assert_eq!(device.name, "Teclado Teste");
        assert_eq!(device.led_count, 3);
        assert_eq!(device.modes.len(), 2);
        assert!(device.modes[0].per_led());
        assert!(device.modes[1].has_speed());
        assert_eq!(device.modes[1].colors, vec![[1, 2, 3]]);
    }
}

#[test]
fn truncated_controller_is_an_error() {
    let data = controller_bytes(4);
    assert!(decode_device(0, &data[..data.len() / 2], 4).is_err());
}

#[test]
fn mode_round_trips() {
    let bytes = mode_bytes("X", FLAG_SPEED, 4);
    let mode = decode_mode(&mut Reader::new(&bytes), 4).unwrap();
    assert_eq!(encode_mode(&mode, 4), bytes);
}

#[test]
fn decodes_profiles_and_colors() {
    assert_eq!(
        decode_profiles(&profiles_bytes()).unwrap(),
        ["Jogo", "Normal"]
    );
    assert_eq!(parse_color("#FF8000"), Some([255, 128, 0]));
    assert_eq!(parse_color("zzzzzz"), None);
    assert_eq!(parse_color("fff"), None);
}

#[tokio::test]
async fn snapshot_lists_devices_and_profiles() {
    let (port, log) = fake_server().await;
    let snapshot = snapshot(port).await.unwrap();
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].name, "Teclado Teste");
    assert_eq!(snapshot.profiles, ["Jogo", "Normal"]);
    assert_eq!(sent(&log, 40)[0], CLIENT_VERSION.to_le_bytes());
    let name = &sent(&log, 50)[0];
    assert!(name.starts_with(crate::channel::app_name().as_bytes()));
}

#[tokio::test]
async fn paint_updates_leds_of_named_device() {
    let (port, log) = fake_server().await;
    let painted = paint(
        port,
        Target::Name("teclado teste".to_string()),
        [10, 20, 30],
    )
    .await
    .unwrap();
    assert_eq!(painted, 1);
    assert_eq!(sent(&log, 1100).len(), 0);
    let leds = &sent(&log, 1050)[0];
    assert_eq!(&leds[..4], &18u32.to_le_bytes());
    assert_eq!(&leds[4..6], &3u16.to_le_bytes());
    assert_eq!(&leds[6..10], &[10, 20, 30, 0]);
    assert!(
        paint(port, Target::Name("outro".to_string()), [0, 0, 0])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn mode_change_clamps_speed_and_sets_color() {
    let (port, log) = fake_server().await;
    set_mode(port, 0, 1, Some(99), Some([5, 6, 7]))
        .await
        .unwrap();
    let packet = &sent(&log, 1101)[0];
    assert_eq!(&packet[4..8], &1u32.to_le_bytes());
    let mode = decode_mode(&mut Reader::new(&packet[8..]), 4).unwrap();
    assert_eq!(mode.name, "Breathing");
    assert_eq!(mode.speed, 5);
    assert_eq!(mode.colors, vec![[5, 6, 7]]);
}

#[tokio::test]
async fn profile_is_loaded_by_name() {
    let (port, log) = fake_server().await;
    apply_profile(port, "Jogo").await.unwrap();
    let data = &sent(&log, 152)[0];
    assert_eq!(&data[..2], &5u16.to_le_bytes());
    assert_eq!(&data[2..], b"Jogo\0");
}

#[tokio::test]
async fn connection_refused_is_reported() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    assert!(matches!(
        snapshot(port).await,
        Err(LedError::Connect { .. })
    ));
}
