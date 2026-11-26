use cpal::ErrorKind;
use std::path::PathBuf;

mod opus_file;
mod playback;
mod recorder;
mod resample;
#[cfg(test)]
mod tests;

pub use playback::Playback;
pub use recorder::Recorder;

#[derive(Debug, Clone, thiserror::Error)]
pub enum AudioError {
    #[error("Nenhum microfone encontrado.")]
    NoInputDevice,
    #[error("Nenhuma saída de áudio encontrada.")]
    NoOutputDevice,
    #[error("Acesso ao áudio negado. Verifique as permissões do sistema.")]
    PermissionDenied,
    #[error("Falha no dispositivo de áudio: {0}")]
    Device(String),
    #[error("Falha ao acessar o arquivo: {0}")]
    Io(String),
    #[error("Formato de áudio não suportado: {0}")]
    Format(String),
    #[error("O áudio passa de 1 hora. Abra o arquivo em outro programa.")]
    TooLong,
    #[error("A gravação não contém áudio.")]
    Empty,
}

#[derive(Debug, Clone)]
pub struct VoiceNote {
    pub path: PathBuf,
    pub seconds: u32,
    pub waveform: Vec<u8>,
}

fn map_cpal(e: cpal::Error, input: bool) -> AudioError {
    match e.kind() {
        ErrorKind::PermissionDenied => AudioError::PermissionDenied,
        ErrorKind::DeviceNotAvailable if input => AudioError::NoInputDevice,
        ErrorKind::DeviceNotAvailable => AudioError::NoOutputDevice,
        _ => AudioError::Device(e.to_string()),
    }
}
