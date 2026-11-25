use std::time::{Duration, Instant};

use hub_core::leds::{self, DeviceKind, Target};

fn kind_of(text: &str) -> Option<DeviceKind> {
    match text {
        "teclado" => Some(DeviceKind::Keyboard),
        "placa" => Some(DeviceKind::Motherboard),
        "memorias" => Some(DeviceKind::Memory),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn step() {
    let who = std::env::var("ALVO").unwrap();
    let action = std::env::var("ACAO").unwrap();
    let hold: u64 = std::env::var("SEGUNDOS").map_or(28, |s| s.parse().unwrap());
    let snapshot = leds::snapshot(6742).await.unwrap();
    let mut names: Vec<String> = snapshot
        .devices
        .iter()
        .filter(|d| kind_of(&who).is_none_or(|k| d.kind == k))
        .map(|d| d.name.clone())
        .collect();
    names.dedup();
    for name in &names {
        let device = snapshot.devices.iter().find(|d| &d.name == name).unwrap();
        let started = Instant::now();
        let result = if let Some(hex) = action.strip_prefix("cor:") {
            leds::paint(6742, Target::Name(name.clone()), leds::parse_color(hex).unwrap())
                .await
                .map(|_| ())
        } else if let Some(rest) = action.strip_prefix("efeito:") {
            let (wanted, level) = rest.split_once(':').unwrap();
            let index = device
                .modes
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(wanted))
                .unwrap_or_else(|| panic!("{name} sem o efeito {wanted}"));
            let mode = &device.modes[index];
            let speed = mode.speed_from_level(level.parse().unwrap(), false);
            leds::set_mode(6742, Target::Name(name.clone()), index, Some(speed), None).await
        } else {
            leds::turn_off(6742, Target::Name(name.clone())).await.map(|_| ())
        };
        println!("{name}: {:?} em {:?}", result, started.elapsed());
    }
    tokio::time::sleep(Duration::from_secs(hold)).await;
    let after = leds::snapshot(6742).await.unwrap();
    for device in after.devices.iter().filter(|d| names.contains(&d.name)) {
        let mode = device.active().unwrap();
        println!(
            "LIDO {} -> modo {} ({}), velocidade {}",
            device.name, mode.name, device.active_mode, mode.speed
        );
    }
}
