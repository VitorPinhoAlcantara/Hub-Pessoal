use std::fs::File;
use std::io::{BufReader, Read, Seek, Write};
use std::path::Path;

use ogg::{PacketReader, PacketWriteEndInfo, PacketWriter};
use ropus::{Application, Bitrate, Channels, DecodeMode, Decoder, Encoder, Signal};

use super::AudioError;
use super::resample::{RATE, Resampler};

pub const FRAME: usize = 960;
const SERIAL: u32 = 0x4855_4231;
const PAGE_EVERY: usize = 25;
const MAX_DECODED: usize = 60 * 60 * RATE as usize;

pub struct OggOpusWriter<W: Write> {
    writer: PacketWriter<'static, W>,
    encoder: Encoder,
    pub pre_skip: u32,
    pending: Option<(Vec<u8>, u64)>,
    packets: usize,
    real_samples: u64,
}

impl<W: Write> OggOpusWriter<W> {
    pub fn new(sink: W) -> Result<Self, AudioError> {
        let encoder = Encoder::builder(RATE, Channels::Mono, Application::Voip)
            .bitrate(Bitrate::Bits(24_000))
            .signal(Signal::Voice)
            .build()
            .map_err(|e| AudioError::Format(e.to_string()))?;
        let pre_skip = encoder.lookahead();
        let mut writer = PacketWriter::new(sink);
        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1);
        head.push(1);
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        let vendor = b"hub-pessoal";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        writer
            .write_packet(head, SERIAL, PacketWriteEndInfo::EndPage, 0)
            .map_err(io_err)?;
        writer
            .write_packet(tags, SERIAL, PacketWriteEndInfo::EndPage, 0)
            .map_err(io_err)?;
        Ok(Self {
            writer,
            encoder,
            pre_skip,
            pending: None,
            packets: 0,
            real_samples: 0,
        })
    }

    pub fn encode_frame(&mut self, pcm: &[f32; FRAME], real: usize) -> Result<(), AudioError> {
        let mut out = [0u8; 1275];
        let n = self
            .encoder
            .encode_float(pcm, &mut out)
            .map_err(|e| AudioError::Format(e.to_string()))?;
        self.real_samples += real as u64;
        self.packets += 1;
        let granule = self.pre_skip as u64 + (self.packets * FRAME) as u64;
        if let Some((data, g)) = self.pending.replace((out[..n].to_vec(), granule)) {
            let end = if self.packets.is_multiple_of(PAGE_EVERY) {
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            self.writer
                .write_packet(data, SERIAL, end, g)
                .map_err(io_err)?;
            if self.packets.is_multiple_of(PAGE_EVERY) {
                self.writer.inner_mut().flush().map_err(io_err)?;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<W, AudioError> {
        let needed = self.real_samples + self.pre_skip as u64;
        let zeros = [0f32; FRAME];
        while ((self.packets * FRAME) as u64) < needed {
            self.encode_frame(&zeros, 0)?;
        }
        if let Some((data, _)) = self.pending.take() {
            let granule = needed;
            self.writer
                .write_packet(data, SERIAL, PacketWriteEndInfo::EndStream, granule)
                .map_err(io_err)?;
        }
        let mut sink = self.writer.into_inner();
        sink.flush().map_err(io_err)?;
        Ok(sink)
    }
}

fn io_err(e: std::io::Error) -> AudioError {
    AudioError::Io(e.to_string())
}

pub fn decode_file(path: &Path) -> Result<Vec<i16>, AudioError> {
    let mut magic = [0u8; 4];
    let mut file = File::open(path).map_err(io_err)?;
    let n = file.read(&mut magic).map_err(io_err)?;
    if n == 4 && &magic == b"OggS" {
        file.rewind().map_err(io_err)?;
        decode_ogg_opus(BufReader::new(file))
    } else {
        decode_other(path)
    }
}

pub fn decode_ogg_opus<R: Read + Seek>(source: R) -> Result<Vec<i16>, AudioError> {
    let mut reader = PacketReader::new(source);
    let mut decoder: Option<(Decoder, usize)> = None;
    let mut pcm = Vec::new();
    let mut skip = 0usize;
    let mut serial = None;
    let mut final_granule: Option<u64> = None;
    let mut buf = vec![0f32; 5760 * 2];
    let mut header_seen = false;
    let mut tags_seen = false;
    while let Ok(Some(packet)) = reader.read_packet() {
        if !header_seen {
            let d = &packet.data;
            if d.len() < 19 || &d[..8] != b"OpusHead" {
                return Err(AudioError::Format("o arquivo OGG não contém Opus".into()));
            }
            let channels = d[9] as usize;
            if d[18] != 0 || !(1..=2).contains(&channels) {
                return Err(AudioError::Format("canais Opus não suportados".into()));
            }
            skip = u16::from_le_bytes([d[10], d[11]]) as usize;
            let layout = if channels == 1 {
                Channels::Mono
            } else {
                Channels::Stereo
            };
            let dec = Decoder::new(RATE, layout).map_err(|e| AudioError::Format(e.to_string()))?;
            decoder = Some((dec, channels));
            serial = Some(packet.stream_serial());
            header_seen = true;
            continue;
        }
        if !tags_seen && packet.data.starts_with(b"OpusTags") {
            tags_seen = true;
            continue;
        }
        if Some(packet.stream_serial()) != serial {
            continue;
        }
        let Some((dec, channels)) = decoder.as_mut() else {
            break;
        };
        let n = match dec.decode_float(
            &packet.data,
            &mut buf[..5760 * *channels],
            DecodeMode::Normal,
        ) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if *channels == 1 {
            pcm.extend(buf[..n].iter().map(|s| to_i16(*s)));
        } else {
            pcm.extend(
                buf[..n * 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| to_i16((c[0] + c[1]) * 0.5)),
            );
        }
        if pcm.len() > MAX_DECODED {
            return Err(AudioError::TooLong);
        }
        if packet.last_in_stream() {
            final_granule = Some(packet.absgp_page());
        }
    }
    if !header_seen {
        return Err(AudioError::Format("arquivo OGG vazio ou inválido".into()));
    }
    let skip = skip.min(pcm.len());
    pcm.drain(..skip);
    if let Some(g) = final_granule {
        let total = g.saturating_sub(skip as u64) as usize;
        if total < pcm.len() {
            pcm.truncate(total);
        }
    }
    Ok(pcm)
}

fn to_i16(s: f32) -> i16 {
    (s * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

fn decode_other(path: &Path) -> Result<Vec<i16>, AudioError> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let fmt = |e: symphonia::core::errors::Error| AudioError::Format(e.to_string());
    let file = File::open(path).map_err(io_err)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(fmt)?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::Format("nenhuma faixa de áudio".into()))?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(fmt)?;
    let mut resampler: Option<(Resampler, u32)> = None;
    let mut pcm = Vec::new();
    let mut mono = Vec::new();
    let mut converted = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        mono.clear();
        mono.extend(
            samples
                .samples()
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
        let rs = match &mut resampler {
            Some((r, rate)) if *rate == spec.rate => r,
            slot => &mut slot.insert((Resampler::new(spec.rate, RATE), spec.rate)).0,
        };
        converted.clear();
        rs.process(&mono, &mut converted);
        pcm.extend(converted.iter().map(|s| to_i16(*s)));
        if pcm.len() > MAX_DECODED {
            return Err(AudioError::TooLong);
        }
    }
    if pcm.is_empty() {
        return Err(AudioError::Format("nenhum áudio decodificado".into()));
    }
    Ok(pcm)
}
