//! Fixed mono Opus inputs and a real per-edge decoder for the native peer.
//! No acoustic device, playout clock or end-to-end latency is represented here.
use crate::archive::{Result, hash};
use rtc::rtp::{Packet, header::Header};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, c_void},
    fs,
    path::Path,
    time::Duration,
};

const MAGIC: &[u8; 8] = b"GPOPUS1\n";
const SAMPLE_RATE: i32 = 48_000;
const SAMPLES: i32 = 960;
const PACKETS: usize = 500;
const PERIOD_NS: u64 = 20_000_000;

#[link(name = "opus")]
unsafe extern "C" {
    fn opus_decoder_create(rate: i32, channels: i32, error: *mut i32) -> *mut c_void;
    fn opus_decoder_destroy(state: *mut c_void);
    fn opus_decode_float(
        state: *mut c_void,
        data: *const u8,
        len: i32,
        pcm: *mut f32,
        size: i32,
        fec: i32,
    ) -> i32;
    fn opus_packet_get_nb_samples(data: *const u8, len: i32, rate: i32) -> i32;
    fn opus_packet_get_nb_channels(data: *const u8) -> i32;
    fn opus_get_version_string() -> *const std::ffi::c_char;
    fn opus_encoder_create(
        rate: i32,
        channels: i32,
        application: i32,
        error: *mut i32,
    ) -> *mut c_void;
    fn opus_encoder_destroy(state: *mut c_void);
    fn opus_encoder_ctl(state: *mut c_void, request: i32, ...) -> i32;
}

fn configured_lookahead() -> Result<i32> {
    let mut error = 0;
    let encoder = unsafe { opus_encoder_create(SAMPLE_RATE, 1, 2049, &mut error) };
    if encoder.is_null() || error != 0 {
        return Err("cannot create configured Opus encoder for lookahead query".into());
    }
    let result = (|| {
        for (request, value) in [
            (4002, 128000),
            (4006, 0),
            (4010, 10),
            (4012, 1),
            (4014, 1),
            (4016, 0),
        ] {
            if unsafe { opus_encoder_ctl(encoder, request, value) } != 0 {
                return Err("Opus setting rejected".into());
            }
        }
        let mut lookahead: i32 = 0;
        if unsafe { opus_encoder_ctl(encoder, 4027, &mut lookahead as *mut i32) } != 0
            || lookahead < 0
        {
            return Err("cannot query actual configured Opus lookahead".into());
        }
        Ok(lookahead)
    })();
    unsafe { opus_encoder_destroy(encoder) };
    result
}

// Same Neumaier sum as CPython 3.13/3.14's float sum in opus-fixture.py.
// Ordinary Iterator::sum changes near-zero PCM bits for the four-tone source.
fn compensated_sum(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut correction) = (0f64, 0f64);
    for value in values {
        let next = sum + value;
        correction += if sum.abs() >= value.abs() {
            (sum - next) + value
        } else {
            (value - next) + sum
        };
        sum = next;
    }
    sum + correction
}

fn source_pcm(kind: &str) -> (Value, Vec<f32>) {
    let (frequencies, gain): (&[i32], f64) = if kind == "mic" {
        (&[317, 719, 1249, 2027], 0.07)
    } else {
        (&[440], 1.0)
    };
    let samples: Vec<f32> = (0..480_000)
        .map(|index| {
            compensated_sum(frequencies.iter().map(|frequency| {
                gain * (2.0 * std::f64::consts::PI * f64::from(*frequency) * index as f64
                    / 48_000.0)
                    .sin()
            })) as f32
        })
        .collect();
    let raw: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0f32, f32::max);
    (
        json!({"kind":kind,"frequencies_hz":frequencies,"gain_per_tone":gain,"phase_at_start_radians":0,
        "samples":samples.len(),"format":"float32le mono","sha256":hash(&raw),"peak":peak}),
        samples,
    )
}

pub struct Decoder(*mut c_void);
// A decoder belongs to one receive task; never shared between tasks/streams.
unsafe impl Send for Decoder {}

pub struct Decoded {
    pub samples: Vec<f32>,
    pub encoded_channels: i32,
}
impl Decoder {
    pub fn new() -> Result<Self> {
        let mut error = 0;
        let state = unsafe { opus_decoder_create(SAMPLE_RATE, 1, &mut error) };
        if state.is_null() || error != 0 {
            return Err(format!("Opus decoder creation failed: {error}").into());
        }
        Ok(Self(state))
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Decoded> {
        if packet.is_empty() || packet.len() > 1500 {
            return Err("invalid Opus packet size".into());
        }
        let duration = unsafe {
            opus_packet_get_nb_samples(packet.as_ptr(), packet.len() as i32, SAMPLE_RATE)
        };
        if duration != SAMPLES {
            return Err(format!("requires actual 20ms Opus packet, got {duration} samples").into());
        }
        let encoded_channels = unsafe { opus_packet_get_nb_channels(packet.as_ptr()) };
        let mut samples = vec![0f32; SAMPLES as usize];
        let decoded = unsafe {
            opus_decode_float(
                self.0,
                packet.as_ptr(),
                packet.len() as i32,
                samples.as_mut_ptr(),
                SAMPLES,
                0,
            )
        };
        if decoded != SAMPLES || samples.iter().any(|sample| !sample.is_finite()) {
            return Err(format!("Opus decode failed/nonfinite: {decoded}").into());
        }
        Ok(Decoded {
            samples,
            encoded_channels,
        })
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { opus_decoder_destroy(self.0) };
    }
}

pub fn decoder_provenance() -> Result<Value> {
    let version = unsafe { CStr::from_ptr(opus_get_version_string()) }.to_str()?;
    let mut paths: Vec<_> = fs::read_to_string("/proc/self/maps")?
        .lines()
        .filter(|line| line.contains("libopus.so"))
        .filter_map(|line| line.split_whitespace().last())
        .filter(|path| path.starts_with('/'))
        .map(|path| fs::canonicalize(path))
        .collect::<std::result::Result<_, _>>()?;
    paths.sort();
    paths.dedup();
    if paths.len() != 1 {
        return Err("cannot identify one actual mapped libopus decoder".into());
    }
    Ok(
        json!({"version":version,"path":paths[0],"sha256":hash(&fs::read(&paths[0])?),
        "output":"48000Hz mono float32; no acoustic device or playout/latency claim"}),
    )
}

pub struct AudioArchive {
    pub metadata: Value,
    pub sha256: String,
    pub packets: Vec<Vec<u8>>,
    pub kind: String,
}
impl AudioArchive {
    pub fn parse(data: &[u8], expected_kind: &str) -> Result<Self> {
        if !matches!(expected_kind, "mic" | "source")
            || !(12..=262_144).contains(&data.len())
            || &data[..8] != MAGIC
        {
            return Err("invalid audio archive/kind".into());
        }
        let length = u32::from_be_bytes(data[8..12].try_into()?) as usize;
        if !(1..=65_536).contains(&length) || 12 + length > data.len() {
            return Err("invalid audio metadata length".into());
        }
        let metadata =
            serde_json::from_slice::<crate::unique_json::UniqueJson>(&data[12..12 + length])?.0;
        for (key, expected) in [
            ("schema", 1),
            ("sample_rate_hz", 48_000),
            ("channels", 1),
            ("packet_duration_ms", 20),
            ("packets", 500),
            ("duration_seconds", 10),
            ("rtp_clock_hz", 48_000),
            ("rtp_timestamp_step", 960),
            ("payload_bitrate_bps", 128_000),
            ("encoded_bytes", 160_000),
        ] {
            if metadata[key].as_u64() != Some(expected) {
                return Err(format!("audio policy differs: {key}").into());
            }
        }
        if metadata["codec"] != "opus"
            || metadata["pcm"]["kind"] != expected_kind
            || metadata["pcm_latency_calibrated"] != false
            || metadata["comparison_available"] != false
        {
            return Err("audio codec/source identity/uncalibrated clock policy differs".into());
        }
        if metadata["encoder"]["settings_readback"]
            != json!({"bitrate":128_000,"vbr":0,"complexity":10,"inband_fec":1,"packet_loss_percent":1,"dtx":0})
        {
            return Err("audio encoder policy differs".into());
        }
        let mut cursor = 12 + length;
        let mut packets = Vec::with_capacity(PACKETS);
        let mut encoded = Vec::with_capacity(160_000);
        let mut decoded: Vec<f32> = Vec::with_capacity(480_000);
        let mut decoder = Decoder::new()?;
        while cursor < data.len() {
            if packets.len() >= PACKETS || cursor + 12 > data.len() {
                return Err("extra/truncated audio record".into());
            }
            let due = u64::from_be_bytes(data[cursor..cursor + 8].try_into()?);
            let size = u32::from_be_bytes(data[cursor + 8..cursor + 12].try_into()?) as usize;
            cursor += 12;
            if due != packets.len() as u64 * PERIOD_NS || size != 320 || cursor + size > data.len()
            {
                return Err("audio packet deadline/length differs".into());
            }
            let packet = &data[cursor..cursor + size];
            cursor += size;
            let pcm = decoder.decode(packet)?;
            if pcm.encoded_channels != 1 {
                return Err("audio archive encodes stereo despite mono policy".into());
            }
            decoded.extend(pcm.samples);
            encoded.extend_from_slice(packet);
            packets.push(packet.to_vec());
        }
        let decoded_bytes: Vec<u8> = decoded
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        if packets.len() != PACKETS
            || metadata["encoded_packets_sha256"].as_str() != Some(hash(&encoded).as_str())
            || metadata["decode_control"]["samples"] != 480_000
            || metadata["decode_control"]["float32le_sha256"].as_str()
                != Some(hash(&decoded_bytes).as_str())
        {
            return Err("missing/altered audio packets or actual decoded PCM differs".into());
        }
        let (policy, original) = source_pcm(expected_kind);
        if metadata["pcm"] != policy {
            return Err(format!("declared PCM differs from regenerated source waveform: expected {policy}; claimed {}", metadata["pcm"]).into());
        }
        let lookahead = configured_lookahead()?;
        if metadata["encoder"]["lookahead_samples"].as_i64() != Some(i64::from(lookahead))
            || metadata["encoder"]["lookahead_ms"].as_f64()
                != Some(f64::from(lookahead) / 48_000.0 * 1000.0)
            || metadata["encoder"]["application"] != "audio"
            || metadata["encoder"]["application_constant"] != 2049
        {
            return Err("configured Opus application/lookahead differs".into());
        }
        let aligned = &decoded[lookahead as usize..];
        let energy = compensated_sum(aligned.iter().map(|sample| f64::from(*sample).powi(2)));
        let input_energy = compensated_sum(
            original[..aligned.len()]
                .iter()
                .map(|sample| f64::from(*sample).powi(2)),
        );
        let correlation = compensated_sum(
            aligned
                .iter()
                .zip(&original)
                .map(|(a, b)| f64::from(*a) * f64::from(*b)),
        ) / (energy * input_energy).sqrt();
        let claimed = metadata["decode_control"]["input_correlation_after_codec_lookahead"]
            .as_f64()
            .ok_or("missing decoded correlation")?;
        let peak = decoded
            .iter()
            .map(|sample| sample.abs())
            .fold(0f32, f32::max);
        if !correlation.is_finite()
            || correlation < 0.98
            || (claimed - correlation).abs() > 1e-12
            || metadata["decode_control"]["peak"].as_f64() != Some(f64::from(peak))
        {
            return Err(format!("actual decoded waveform/peak/correlation differs: correlation {correlation} vs {claimed}, peak {peak} vs {}",metadata["decode_control"]["peak"]).into());
        }
        let decoder = decoder_provenance()?;
        if metadata["provenance"]["library_sha256"] != decoder["sha256"]
            || metadata["provenance"]["library_version"] != decoder["version"]
        {
            return Err("executed decoder library differs from frozen source library".into());
        }
        Ok(Self {
            metadata,
            sha256: hash(data),
            packets,
            kind: expected_kind.to_owned(),
        })
    }
    pub fn read(path: &Path, kind: &str) -> Result<Self> {
        Self::parse(&fs::read(path)?, kind)
    }
    pub fn packet(&self, index: usize, cycle: u64, ssrc: u32) -> Packet {
        let ordinal = cycle * PACKETS as u64 + index as u64;
        Packet {
            header: Header {
                version: 2,
                payload_type: 111,
                sequence_number: ordinal as u16,
                timestamp: (ordinal * SAMPLES as u64) as u32,
                ssrc,
                marker: index == 0 && cycle == 0,
                ..Default::default()
            },
            payload: self.packets[index].clone().into(),
        }
    }
    pub fn due(index: usize, cycle: u64) -> Duration {
        Duration::from_nanos((cycle * PACKETS as u64 + index as u64) * PERIOD_NS)
    }
}
