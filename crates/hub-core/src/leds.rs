use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const CLIENT_VERSION: u32 = 4;
pub const DEFAULT_PORT: u16 = 6742;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_PACKET: usize = 16 * 1024 * 1024;

const ID_CONTROLLER_COUNT: u32 = 0;
const ID_CONTROLLER_DATA: u32 = 1;
const ID_PROTOCOL_VERSION: u32 = 40;
const ID_CLIENT_NAME: u32 = 50;
const ID_PROFILE_LIST: u32 = 150;
const ID_LOAD_PROFILE: u32 = 152;
const ID_UPDATE_LEDS: u32 = 1050;
const ID_SET_CUSTOM_MODE: u32 = 1100;
const ID_UPDATE_MODE: u32 = 1101;

pub const FLAG_SPEED: u32 = 1 << 0;
pub const FLAG_PER_LED_COLOR: u32 = 1 << 5;
pub const FLAG_MODE_COLOR: u32 = 1 << 6;

pub type Rgb = [u8; 3];

#[derive(Debug, thiserror::Error)]
pub enum LedError {
    #[error("não foi possível conectar ao OpenRGB na porta {port}: {reason}")]
    Connect { port: u16, reason: String },
    #[error("o OpenRGB não respondeu a tempo")]
    Timeout,
    #[error("falha de comunicação com o OpenRGB: {0}")]
    Io(#[from] std::io::Error),
    #[error("resposta inválida do OpenRGB: {0}")]
    Protocol(String),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Motherboard,
    Memory,
    Gpu,
    Cooler,
    LedStrip,
    Keyboard,
    Mouse,
    Mousemat,
    Headset,
    HeadsetStand,
    Gamepad,
    Light,
    Speaker,
    Virtual,
    Storage,
    Case,
    Microphone,
    Accessory,
    Keypad,
    Laptop,
    Monitor,
    Other,
}

impl DeviceKind {
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => Self::Motherboard,
            1 => Self::Memory,
            2 => Self::Gpu,
            3 => Self::Cooler,
            4 => Self::LedStrip,
            5 => Self::Keyboard,
            6 => Self::Mouse,
            7 => Self::Mousemat,
            8 => Self::Headset,
            9 => Self::HeadsetStand,
            10 => Self::Gamepad,
            11 => Self::Light,
            12 => Self::Speaker,
            13 => Self::Virtual,
            14 => Self::Storage,
            15 => Self::Case,
            16 => Self::Microphone,
            17 => Self::Accessory,
            18 => Self::Keypad,
            19 => Self::Laptop,
            20 => Self::Monitor,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Motherboard => "Placa mãe",
            Self::Memory => "Memória RAM",
            Self::Gpu => "Placa de vídeo",
            Self::Cooler => "Cooler",
            Self::LedStrip => "Fita de LED",
            Self::Keyboard => "Teclado",
            Self::Mouse => "Mouse",
            Self::Mousemat => "Mousepad",
            Self::Headset => "Headset",
            Self::HeadsetStand => "Suporte de headset",
            Self::Gamepad => "Controle",
            Self::Light => "Luz",
            Self::Speaker => "Caixa de som",
            Self::Virtual => "Virtual",
            Self::Storage => "Armazenamento",
            Self::Case => "Gabinete",
            Self::Microphone => "Microfone",
            Self::Accessory => "Acessório",
            Self::Keypad => "Teclado numérico",
            Self::Laptop => "Notebook",
            Self::Monitor => "Monitor",
            Self::Other => "Dispositivo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    pub name: String,
    pub value: u32,
    pub flags: u32,
    pub speed_min: u32,
    pub speed_max: u32,
    pub brightness_min: u32,
    pub brightness_max: u32,
    pub colors_min: u32,
    pub colors_max: u32,
    pub speed: u32,
    pub brightness: u32,
    pub direction: u32,
    pub color_mode: u32,
    pub colors: Vec<Rgb>,
}

impl Mode {
    pub fn has_speed(&self) -> bool {
        self.flags & FLAG_SPEED != 0 && self.speed_max > self.speed_min
    }

    pub fn per_led(&self) -> bool {
        self.flags & FLAG_PER_LED_COLOR != 0
    }

    pub fn mode_color(&self) -> bool {
        self.flags & FLAG_MODE_COLOR != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub index: u32,
    pub kind: DeviceKind,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub serial: String,
    pub location: String,
    pub active_mode: usize,
    pub modes: Vec<Mode>,
    pub led_count: usize,
}

impl Device {
    pub fn active(&self) -> Option<&Mode> {
        self.modes.get(self.active_mode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    All,
    Index(u32),
    Name(String),
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], LedError> {
        let end = self
            .pos
            .checked_add(count)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| LedError::Protocol("dados truncados".to_string()))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16, LedError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, LedError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn string(&mut self) -> Result<String, LedError> {
        let length = usize::from(self.u16()?);
        let bytes = self.take(length)?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    fn color(&mut self) -> Result<Rgb, LedError> {
        let bytes = self.take(4)?;
        Ok([bytes[0], bytes[1], bytes[2]])
    }
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&((value.len() + 1) as u16).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_color(out: &mut Vec<u8>, color: Rgb) {
    out.extend_from_slice(&[color[0], color[1], color[2], 0]);
}

pub fn encode_packet(device: u32, id: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + data.len());
    out.extend_from_slice(b"ORGB");
    put_u32(&mut out, device);
    put_u32(&mut out, id);
    put_u32(&mut out, data.len() as u32);
    out.extend_from_slice(data);
    out
}

pub fn encode_mode(mode: &Mode, version: u32) -> Vec<u8> {
    let mut out = Vec::new();
    put_string(&mut out, &mode.name);
    put_u32(&mut out, mode.value);
    put_u32(&mut out, mode.flags);
    put_u32(&mut out, mode.speed_min);
    put_u32(&mut out, mode.speed_max);
    if version >= 3 {
        put_u32(&mut out, mode.brightness_min);
        put_u32(&mut out, mode.brightness_max);
    }
    put_u32(&mut out, mode.colors_min);
    put_u32(&mut out, mode.colors_max);
    put_u32(&mut out, mode.speed);
    if version >= 3 {
        put_u32(&mut out, mode.brightness);
    }
    put_u32(&mut out, mode.direction);
    put_u32(&mut out, mode.color_mode);
    out.extend_from_slice(&(mode.colors.len() as u16).to_le_bytes());
    for color in &mode.colors {
        put_color(&mut out, *color);
    }
    out
}

fn decode_mode(reader: &mut Reader, version: u32) -> Result<Mode, LedError> {
    let name = reader.string()?;
    let value = reader.u32()?;
    let flags = reader.u32()?;
    let speed_min = reader.u32()?;
    let speed_max = reader.u32()?;
    let (brightness_min, brightness_max) = if version >= 3 {
        (reader.u32()?, reader.u32()?)
    } else {
        (0, 0)
    };
    let colors_min = reader.u32()?;
    let colors_max = reader.u32()?;
    let speed = reader.u32()?;
    let brightness = if version >= 3 { reader.u32()? } else { 0 };
    let direction = reader.u32()?;
    let color_mode = reader.u32()?;
    let count = reader.u16()?;
    let colors = (0..count)
        .map(|_| reader.color())
        .collect::<Result<_, _>>()?;
    Ok(Mode {
        name,
        value,
        flags,
        speed_min,
        speed_max,
        brightness_min,
        brightness_max,
        colors_min,
        colors_max,
        speed,
        brightness,
        direction,
        color_mode,
        colors,
    })
}

pub fn decode_device(index: u32, data: &[u8], version: u32) -> Result<Device, LedError> {
    let mut reader = Reader::new(data);
    reader.u32()?;
    let kind = DeviceKind::from_code(reader.u32()?);
    let name = reader.string()?;
    let vendor = if version >= 1 {
        reader.string()?
    } else {
        String::new()
    };
    let description = reader.string()?;
    reader.string()?;
    let serial = reader.string()?;
    let location = reader.string()?;
    let mode_count = reader.u16()?;
    let active_mode = reader.u32()? as usize;
    let modes = (0..mode_count)
        .map(|_| decode_mode(&mut reader, version))
        .collect::<Result<Vec<_>, _>>()?;
    let zone_count = reader.u16()?;
    for _ in 0..zone_count {
        reader.string()?;
        reader.take(16)?;
        let matrix = usize::from(reader.u16()?);
        reader.take(matrix)?;
        if version >= 4 {
            let segments = reader.u16()?;
            for _ in 0..segments {
                reader.string()?;
                reader.take(12)?;
            }
        }
    }
    let led_count = usize::from(reader.u16()?);
    Ok(Device {
        index,
        kind,
        name,
        vendor,
        description,
        serial,
        location,
        active_mode,
        modes,
        led_count,
    })
}

pub fn decode_profiles(data: &[u8]) -> Result<Vec<String>, LedError> {
    let mut reader = Reader::new(data);
    reader.u32()?;
    let count = reader.u16()?;
    (0..count).map(|_| reader.string()).collect()
}

pub struct Client {
    stream: TcpStream,
    version: u32,
}

impl Client {
    pub async fn connect(port: u16) -> Result<Self, LedError> {
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
            .await
            .map_err(|_| LedError::Connect {
                port,
                reason: "tempo esgotado".to_string(),
            })?
            .map_err(|error| LedError::Connect {
                port,
                reason: error.to_string(),
            })?;
        stream.set_nodelay(true)?;
        let mut client = Self { stream, version: 0 };
        client
            .write(0, ID_PROTOCOL_VERSION, &CLIENT_VERSION.to_le_bytes())
            .await?;
        client.version = match client.read_until(ID_PROTOCOL_VERSION).await {
            Ok(data) => Reader::new(&data).u32()?.min(CLIENT_VERSION),
            Err(LedError::Timeout) => 0,
            Err(error) => return Err(error),
        };
        let mut name = crate::channel::app_name().as_bytes().to_vec();
        name.push(0);
        client.write(0, ID_CLIENT_NAME, &name).await?;
        Ok(client)
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    async fn write(&mut self, device: u32, id: u32, data: &[u8]) -> Result<(), LedError> {
        let packet = encode_packet(device, id, data);
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            self.stream.write_all(&packet).await?;
            self.stream.flush().await
        })
        .await
        .map_err(|_| LedError::Timeout)??;
        Ok(())
    }

    async fn read_until(&mut self, expected: u32) -> Result<Vec<u8>, LedError> {
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            loop {
                let mut header = [0u8; 16];
                self.stream.read_exact(&mut header).await?;
                if &header[..4] != b"ORGB" {
                    return Err(LedError::Protocol("cabeçalho inválido".to_string()));
                }
                let id = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
                let size =
                    u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
                if size > MAX_PACKET {
                    return Err(LedError::Protocol("pacote grande demais".to_string()));
                }
                let mut data = vec![0u8; size];
                self.stream.read_exact(&mut data).await?;
                if id == expected {
                    return Ok(data);
                }
            }
        })
        .await
        .map_err(|_| LedError::Timeout)?
    }

    async fn request(&mut self, device: u32, id: u32, data: &[u8]) -> Result<Vec<u8>, LedError> {
        self.write(device, id, data).await?;
        self.read_until(id).await
    }

    async fn sync(&mut self) -> Result<(), LedError> {
        self.request(0, ID_CONTROLLER_COUNT, &[]).await.map(|_| ())
    }

    pub async fn devices(&mut self) -> Result<Vec<Device>, LedError> {
        let count = Reader::new(&self.request(0, ID_CONTROLLER_COUNT, &[]).await?).u32()?;
        let mut devices = Vec::new();
        for index in 0..count {
            let request = self.version.to_le_bytes();
            let data = self.request(index, ID_CONTROLLER_DATA, &request).await?;
            devices.push(decode_device(index, &data, self.version)?);
        }
        Ok(devices)
    }

    pub async fn profiles(&mut self) -> Result<Vec<String>, LedError> {
        if self.version < 2 {
            return Ok(Vec::new());
        }
        let data = self.request(0, ID_PROFILE_LIST, &[]).await?;
        decode_profiles(&data)
    }

    pub async fn load_profile(&mut self, name: &str) -> Result<(), LedError> {
        let mut data = Vec::new();
        put_string(&mut data, name);
        self.write(0, ID_LOAD_PROFILE, &data).await?;
        self.sync().await
    }

    async fn send_mode(
        &mut self,
        device: &Device,
        index: usize,
        mode: &Mode,
    ) -> Result<(), LedError> {
        let body = encode_mode(mode, self.version);
        let mut data = Vec::new();
        put_u32(&mut data, (8 + body.len()) as u32);
        put_u32(&mut data, index as u32);
        data.extend_from_slice(&body);
        self.write(device.index, ID_UPDATE_MODE, &data).await
    }

    pub async fn set_color(&mut self, device: &Device, color: Rgb) -> Result<(), LedError> {
        let active_per_led = device.active().is_some_and(Mode::per_led);
        if !active_per_led && device.modes.iter().any(Mode::per_led) {
            self.write(device.index, ID_SET_CUSTOM_MODE, &[]).await?;
        } else if !active_per_led {
            let (index, mode) = device
                .modes
                .iter()
                .enumerate()
                .find(|(_, mode)| mode.mode_color())
                .ok_or_else(|| {
                    LedError::Invalid(format!("{} não aceita cor sólida", device.name))
                })?;
            let mode = with_color(mode, color);
            self.send_mode(device, index, &mode).await?;
            return self.sync().await;
        }
        let size = 6 + device.led_count * 4;
        let mut data = Vec::with_capacity(size);
        put_u32(&mut data, size as u32);
        data.extend_from_slice(&(device.led_count as u16).to_le_bytes());
        for _ in 0..device.led_count {
            put_color(&mut data, color);
        }
        self.write(device.index, ID_UPDATE_LEDS, &data).await?;
        self.sync().await
    }

    pub async fn activate_mode(
        &mut self,
        device: &Device,
        index: usize,
        speed: Option<u32>,
        color: Option<Rgb>,
    ) -> Result<(), LedError> {
        let base = device
            .modes
            .get(index)
            .ok_or_else(|| LedError::Invalid("efeito inexistente".to_string()))?;
        let mut mode = match color {
            Some(color) if base.mode_color() => with_color(base, color),
            _ => base.clone(),
        };
        if let Some(speed) = speed
            && mode.has_speed()
        {
            mode.speed = speed.clamp(mode.speed_min, mode.speed_max);
        }
        self.send_mode(device, index, &mode).await?;
        self.sync().await
    }
}

fn with_color(mode: &Mode, color: Rgb) -> Mode {
    let wanted = mode.colors.len().max(mode.colors_min as usize).max(1);
    let count = if mode.colors_max > 0 {
        wanted.min(mode.colors_max as usize)
    } else {
        wanted
    };
    let mut mode = mode.clone();
    mode.colors = vec![color; count];
    mode
}

fn matches(device: &Device, target: &Target) -> bool {
    match target {
        Target::All => true,
        Target::Index(index) => device.index == *index,
        Target::Name(name) => device.name.to_lowercase() == name.trim().to_lowercase(),
    }
}

pub async fn snapshot(port: u16) -> Result<Snapshot, LedError> {
    let mut client = Client::connect(port).await?;
    let devices = client.devices().await?;
    let profiles = client.profiles().await.unwrap_or_default();
    Ok(Snapshot { devices, profiles })
}

pub async fn paint(port: u16, target: Target, color: Rgb) -> Result<usize, LedError> {
    let mut client = Client::connect(port).await?;
    let devices = client.devices().await?;
    let mut painted = 0;
    let mut last_error = None;
    for device in devices.iter().filter(|d| matches(d, &target)) {
        match client.set_color(device, color).await {
            Ok(()) => painted += 1,
            Err(error) => last_error = Some(error),
        }
    }
    match (painted, last_error) {
        (0, Some(error)) => Err(error),
        (0, None) => Err(LedError::Invalid("dispositivo não encontrado".to_string())),
        _ => Ok(painted),
    }
}

pub async fn set_mode(
    port: u16,
    device: u32,
    mode: usize,
    speed: Option<u32>,
    color: Option<Rgb>,
) -> Result<(), LedError> {
    let mut client = Client::connect(port).await?;
    let devices = client.devices().await?;
    let device = devices
        .iter()
        .find(|d| d.index == device)
        .ok_or_else(|| LedError::Invalid("dispositivo não encontrado".to_string()))?;
    client.activate_mode(device, mode, speed, color).await
}

pub async fn apply_profile(port: u16, name: &str) -> Result<(), LedError> {
    Client::connect(port).await?.load_profile(name).await
}

pub fn find_openrgb() -> Option<PathBuf> {
    crate::platform::find_program("OpenRGB")
}

pub fn start_openrgb(path: &Path) -> std::io::Result<()> {
    crate::platform::spawn_detached(path, &["--server", "--startminimized"])
}

pub fn parse_color(text: &str) -> Option<Rgb> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

#[cfg(test)]
mod tests;
