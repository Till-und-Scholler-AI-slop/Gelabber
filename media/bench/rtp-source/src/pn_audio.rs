//! Strict finite PN archive import. Full source regeneration and real decode;
//! offline marker/sample integrity only, never a native/browser clock proof.
use crate::{
    archive::{Result, hash},
    audio::{self, AudioArchive, Decoder},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const RATE: u64 = 48_000;
const LENGTH: usize = 6048;
const PERIOD: u64 = 96_000;

pub fn shared_pair(mic: &AudioArchive, source: &AudioArchive) -> Result<()> {
    if mic.unlooped() != source.unlooped()
        || mic.unlooped()
            && (mic.metadata["pn"]["shared_run"] != source.metadata["pn"]["shared_run"]
                || mic.metadata["pn"]["codebook_sha256"]
                    != source.metadata["pn"]["codebook_sha256"]
                || mic.metadata["duration_seconds"] != source.metadata["duration_seconds"])
    {
        return Err(
            "native audio archives must share the same version, whole-run book and duration".into(),
        );
    }
    Ok(())
}
pub fn start_policy(request: &Value, mic: &AudioArchive, test_enabled: bool) -> Result<(u64, u64)> {
    if !mic.unlooped() {
        if request.get("audio_hold_ms").is_some()
            || request.get("total_seconds").is_some()
            || test_enabled
        {
            return Err("legacy V1 replay has no finite/hold test policy".into());
        }
        let seconds = request["seconds"].as_u64().ok_or("duration required")?;
        if seconds == 0 || seconds > 3600 || seconds % 10 != 0 {
            return Err("duration must be whole ten-second periods <=3600".into());
        }
        return Ok((seconds, 0));
    }
    let total = request["total_seconds"]
        .as_u64()
        .ok_or("finite V2 total_seconds including tail required")?;
    if ["seconds", "loop", "loops", "rewind", "endOrdinal"]
        .iter()
        .any(|key| request.get(key).is_some())
        || mic.metadata["duration_seconds"].as_u64() != Some(total)
    {
        return Err("finite V2 duration must exactly equal archive total including tail; no seconds/loop/cut".into());
    }
    let hold = match request.get("audio_hold_ms") {
        Some(value) if test_enabled => value
            .as_u64()
            .filter(|n| [0, 50, 200, 500].contains(n))
            .ok_or("test hold must be 0/50/200/500ms")?,
        Some(_) => {
            return Err(
                "audio hold requires explicit --allow-test-audio-hold before --peer0".into(),
            );
        }
        None => 0,
    };
    Ok((total, hold))
}

fn policy() -> Value {
    json!({"sampleRate":48000,"carrierHz":2000,"chipFrames":96,"chips":63,
        "periodFrames":96000,"amplitude":0.35,"threshold":0.72})
}
fn code(uid: u32, sequence: u32) -> Vec<i32> {
    let mut seed = (uid + 1).wrapping_mul(0x9e3779b1) ^ sequence.wrapping_mul(0x85ebca6b);
    (0..63)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            if seed & 1 == 1 { 1 } else { -1 }
        })
        .collect()
}
fn book(kind: &str, seconds: u64, run: &str) -> Result<Value> {
    if !matches!(kind, "mic" | "source")
        || !(20..=360).contains(&seconds)
        || run.len() != 36
        || !run.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
    {
        return Err("invalid PN source/duration/shared UUID".into());
    }
    let ordinals: Vec<u64> = (RATE..seconds * RATE - LENGTH as u64)
        .step_by(PERIOD as usize)
        .collect();
    let mut signatures = BTreeSet::new();
    let mut sources = Vec::new();
    for (role, uid, ssrc) in [("mic", 0, 0x474d4943u32), ("source", 64, 0x47534130u32)] {
        let mut markers = Vec::new();
        for (sequence, ordinal) in ordinals.iter().enumerate() {
            let signs = code(uid, sequence as u32);
            let inverse: Vec<i32> = signs.iter().map(|v| -v).collect();
            if !signatures.insert(signs.clone().min(inverse)) {
                return Err("duplicate/inverted PN code in shared run".into());
            }
            markers.push(json!({"sequence":sequence,"source_sample_ordinal":ordinal,"code":signs}));
        }
        sources.push(json!({"kind":role,"source_uid":uid,"ssrc":ssrc,"markers":markers}));
    }
    let shared = json!({"run_id":run,"measurement_end_sample_ordinal":seconds*RATE,
        "tail_samples":RATE,"marker_policy":policy(),"sources":sources});
    let own = &shared["sources"][if kind == "mic" { 0 } else { 1 }];
    Ok(
        json!({"run_id":run,"codebook_sha256":hash(&serde_json::to_vec(&shared)?),
        "source_uid":own["source_uid"],"ssrc":own["ssrc"],"marker_policy":policy(),
        "markers":own["markers"],"shared_run":shared}),
    )
}
fn waveform(kind: &str, seconds: u64, book: &Value) -> (Value, Vec<f32>) {
    let (tones, gain): (&[i32], f64) = if kind == "mic" {
        (&[317, 719, 1249, 2027], 0.07)
    } else {
        (&[440], 0.45)
    };
    let mut samples: Vec<f32> = (0..(seconds + 1) * RATE)
        .map(|index| {
            audio::compensated_sum(tones.iter().map(|tone| {
                gain * (2.0 * std::f64::consts::PI * f64::from(*tone) * index as f64 / RATE as f64)
                    .sin()
            })) as f32
        })
        .collect();
    for marker in book["markers"].as_array().unwrap() {
        let start = marker["source_sample_ordinal"].as_u64().unwrap() as usize;
        for offset in 0..LENGTH {
            // Python array('f') rounds the base tones before adding the PN double.
            let sign = marker["code"][offset / 96].as_i64().unwrap() as f64;
            samples[start + offset] = (f64::from(samples[start + offset])
                + sign
                    * (offset as f64 * 2.0 * std::f64::consts::PI * 2000.0 / RATE as f64).sin()
                    * 0.35) as f32;
        }
    }
    let peak = samples.iter().map(|v| v.abs()).fold(0f32, f32::max);
    let raw: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    (
        json!({"kind":kind,"source_uid":book["source_uid"],"frequencies_hz":tones,
        "gain_per_tone":gain,"samples":samples.len(),"format":"float32le mono",
        "sha256":hash(&raw),"peak":peak}),
        samples,
    )
}

#[derive(Clone)]
struct Peak {
    frame: i64,
    sequence: u32,
    score: f64,
    amplitude: f64,
}
// Same complex-demodulation/chip filter as the frozen JS kernel. Distinct peaks
// remain visible; only the trailing 10ms correlation lobe is deduplicated.
fn peaks(samples: &[f32], uid: u32, first: u64) -> Vec<Peak> {
    let (mut raw_i, mut raw_q) = ([0f64; 24], [0f64; 24]);
    let (mut history_i, mut history_q) = ([0f64; 256], [0f64; 256]);
    let (mut sum_i, mut sum_q, mut entries, mut refractory) = (0f64, 0f64, 0usize, 0usize);
    let mut candidate: Option<(usize, Peak)> = None;
    let mut result = Vec::new();
    let (mut current, mut weights, mut weight_energy) = (u32::MAX, Vec::new(), 0f64);
    for frame in (0..samples.len()).step_by(4) {
        let down = frame / 4;
        let slot = down % 24;
        let phase = frame as f64 * 2.0 * std::f64::consts::PI * 2000.0 / RATE as f64;
        let (i, q) = (
            f64::from(samples[frame]) * phase.cos(),
            f64::from(samples[frame]) * phase.sin(),
        );
        sum_i += i - raw_i[slot];
        sum_q += q - raw_q[slot];
        raw_i[slot] = i;
        raw_q[slot] = q;
        if down % 6 != 0 {
            continue;
        }
        let index = entries % 256;
        entries += 1;
        history_i[index] = sum_i;
        history_q[index] = sum_q;
        if entries < 253 || down < refractory || frame < (LENGTH as u64 + first) as usize {
            continue;
        }
        let sequence = ((frame as u64 - LENGTH as u64 - first) / PERIOD) as u32;
        if current != sequence {
            let signs = code(uid, sequence);
            let mean = signs.iter().sum::<i32>() as f64 / 63.0;
            weights = signs.iter().map(|v| f64::from(*v) - mean).collect();
            weight_energy = weights.iter().map(|v| v * v).sum();
            current = sequence;
        }
        let (mut real, mut imaginary, mut energy) = (0f64, 0f64, 0f64);
        for (chip, weight) in weights.iter().enumerate() {
            let slot = (index + 256 - (62 - chip) * 4) % 256;
            let (i, q) = (history_i[slot], history_q[slot]);
            real += i * weight;
            imaginary += q * weight;
            energy += i * i + q * q;
        }
        let score = if energy > 0.0 {
            real.hypot(imaginary) / (energy * weight_energy).sqrt()
        } else {
            0.0
        };
        let amplitude = (energy / 63.0).sqrt() / 12.0;
        if candidate.is_none() && score >= 0.72 && amplitude >= 0.04 {
            candidate = Some((
                down + 120,
                Peak {
                    frame: down as i64,
                    sequence,
                    score: 0.0,
                    amplitude,
                },
            ));
        }
        if let Some((until, best)) = &mut candidate {
            if score > best.score {
                best.score = score;
                best.frame = down as i64;
                best.amplitude = amplitude;
            }
            if down >= *until {
                let mut best = best.clone();
                best.frame = (best.frame - 63 * 24 + 1) * 4;
                result.push(best);
                candidate = None;
                refractory = down + 120;
            }
        }
    }
    result
}
fn verify_markers(decoded: &[f32], book: &Value, lookahead: u64, claimed: &Value) -> Result<()> {
    // The original offline control's provenance, repeated independently here;
    // native executes the Rust detector, so this does not claim a Node invocation.
    if claimed["executed_node"]
        != json!({"version":"v26.8.2","sha256":"8a22a371fd85aecf5411636574309f6380fbc42694aaf0651a089a8ef9c44e52"})
    {
        return Err("frozen offline marker Node version/binary differs".into());
    }
    let expected = book["markers"].as_array().ok_or("missing marker book")?;
    let actual = peaks(
        decoded,
        book["source_uid"].as_u64().unwrap() as u32,
        RATE + lookahead,
    );
    if actual.len() != expected.len()
        || claimed["samples"].as_u64() != Some(decoded.len() as u64)
        || claimed["markers"].as_u64() != Some(expected.len() as u64)
        || claimed["checks"].as_array().map(Vec::len) != Some(expected.len())
    {
        return Err("missing/ambiguous decoded PN markers".into());
    }
    let mut maximum = 0;
    for (index, marker) in expected.iter().enumerate() {
        let matching: Vec<_> = actual
            .iter()
            .filter(|p| p.sequence == index as u32)
            .collect();
        if matching.len() != 1 {
            return Err("missing/ambiguous decoded marker sequence".into());
        }
        let peak = matching[0];
        let ordinal = marker["source_sample_ordinal"].as_i64().unwrap() + lookahead as i64;
        let residual = peak.frame - ordinal;
        maximum = maximum.max(residual.abs());
        let row = &claimed["checks"][index];
        if residual.abs() > 96
            || row["sequence"].as_u64() != Some(index as u64)
            || row["expected_decoded_sample_ordinal"].as_i64() != Some(ordinal)
            || row["actual_decoded_sample_ordinal"].as_i64() != Some(peak.frame)
            || row["residual_samples"].as_i64() != Some(residual)
            || row["score"]
                .as_f64()
                .is_none_or(|v| (v - peak.score).abs() > 1e-10)
            || row["amplitude"]
                .as_f64()
                .is_none_or(|v| (v - peak.amplitude).abs() > 1e-10)
        {
            return Err("actual decoded PN alignment/peak claim differs".into());
        }
    }
    if claimed["max_alignment_error_samples"].as_i64() != Some(maximum) {
        return Err("decoded marker maximum differs".into());
    }
    Ok(())
}

pub fn parse(data: &[u8], kind: &str) -> Result<AudioArchive> {
    if !(12..=8 * 1024 * 1024).contains(&data.len()) || &data[..8] != b"GPOPUS2\n" {
        return Err("invalid finite PN archive length/magic".into());
    }
    let length = u32::from_be_bytes(data[8..12].try_into()?) as usize;
    if !(1..=256 * 1024).contains(&length) || 12 + length > data.len() {
        return Err("invalid finite PN manifest length".into());
    }
    let metadata =
        serde_json::from_slice::<crate::unique_json::UniqueJson>(&data[12..12 + length])?.0;
    let seconds = metadata["measurement_seconds"]
        .as_u64()
        .ok_or("PN measurement duration required")?;
    let book = book(
        kind,
        seconds,
        metadata["run_id"]
            .as_str()
            .ok_or("shared run UUID required")?,
    )?;
    let count = (seconds + 1) * 50;
    for (key, value) in [
        ("schema", 2),
        ("sample_rate_hz", RATE),
        ("channels", 1),
        ("packet_duration_ms", 20),
        ("packets", count),
        ("duration_seconds", seconds + 1),
        ("tail_seconds", 1),
        ("tail_samples", RATE),
        ("measurement_end_sample_ordinal", seconds * RATE),
        ("rtp_clock_hz", RATE),
        ("rtp_timestamp_step", 960),
        ("payload_bitrate_bps", 128000),
        ("encoded_bytes", count * 320),
    ] {
        if metadata[key].as_u64() != Some(value) {
            return Err(format!("finite PN policy differs: {key}").into());
        }
    }
    if metadata["codec"] != "opus"
        || metadata["pn"] != book
        || metadata["loop_policy"] != "unlooped; stop at archive end; no rewind or modulo"
        || metadata["pcm_latency_calibrated"] != false
        || metadata["comparison_available"] != false
    {
        return Err("finite PN source/codebook/uncalibrated policy differs".into());
    }
    let mut packets = Vec::with_capacity(count as usize);
    let mut encoded = Vec::new();
    let mut decoded = Vec::new();
    let mut decoder = Decoder::new()?;
    let mut cursor = 12 + length;
    while cursor < data.len() {
        if packets.len() >= count as usize || cursor + 12 > data.len() {
            return Err("extra/truncated PN record".into());
        }
        let due = u64::from_be_bytes(data[cursor..cursor + 8].try_into()?);
        let size = u32::from_be_bytes(data[cursor + 8..cursor + 12].try_into()?) as usize;
        cursor += 12;
        if due != packets.len() as u64 * 20_000_000 || size != 320 || cursor + size > data.len() {
            return Err("PN packet length/deadline differs".into());
        }
        let packet = &data[cursor..cursor + size];
        cursor += size;
        let pcm = decoder.decode(packet)?;
        if pcm.encoded_channels != 1 {
            return Err("PN archive actually encodes stereo".into());
        }
        decoded.extend(pcm.samples);
        encoded.extend_from_slice(packet);
        packets.push(packet.to_vec());
    }
    let raw: Vec<u8> = decoded.iter().flat_map(|v| v.to_le_bytes()).collect();
    if packets.len() != count as usize
        || metadata["encoded_packets_sha256"] != hash(&encoded)
        || metadata["decode_control"]["samples"].as_u64() != Some((seconds + 1) * RATE)
        || metadata["decode_control"]["float32le_sha256"] != hash(&raw)
    {
        return Err("actual PN packets/full decode hash differs".into());
    }
    let (pcm, original) = waveform(kind, seconds, &book);
    if metadata["pcm"] != pcm {
        return Err("PN source differs from actual regenerated waveform".into());
    }
    let lookahead = audio::configured_lookahead()?;
    let encoder = &metadata["encoder"];
    if encoder["settings_readback"]
        != json!({"bitrate":128000,"vbr":0,"complexity":10,"inband_fec":1,"packet_loss_percent":1,"dtx":0})
        || encoder["application"] != "audio"
        || encoder["application_constant"] != 2049
        || encoder["lookahead_samples"].as_i64() != Some(i64::from(lookahead))
        || encoder["lookahead_ms"].as_f64() != Some(f64::from(lookahead) / RATE as f64 * 1000.0)
    {
        return Err("actual PN encoder/lookahead differs".into());
    }
    let aligned = &decoded[lookahead as usize..];
    let energy = audio::compensated_sum(aligned.iter().map(|v| f64::from(*v).powi(2)));
    let input_energy = audio::compensated_sum(
        original[..aligned.len()]
            .iter()
            .map(|v| f64::from(*v).powi(2)),
    );
    let correlation = audio::compensated_sum(
        aligned
            .iter()
            .zip(&original)
            .map(|(a, b)| f64::from(*a) * f64::from(*b)),
    ) / (energy * input_energy).sqrt();
    let peak = decoded.iter().map(|v| v.abs()).fold(0f32, f32::max);
    if !correlation.is_finite()
        || correlation < 0.98
        || peak >= 0.999
        || pcm["peak"].as_f64().unwrap() >= 0.999
        || metadata["decode_control"]["input_correlation_after_codec_lookahead"]
            .as_f64()
            .is_none_or(|v| (v - correlation).abs() > 1e-12)
        || metadata["decode_control"]["peak"].as_f64() != Some(f64::from(peak))
    {
        return Err("actual PN decoded correlation/clipping differs".into());
    }
    let provenance = audio::decoder_provenance()?;
    if provenance["version"] != "libopus 1.6.1"
        || metadata["provenance"]["library_version"] != provenance["version"]
        || metadata["provenance"]["library_sha256"] != provenance["sha256"]
    {
        return Err("actual PN libopus differs".into());
    }
    for (key, bytes) in [
        (
            "script_sha256",
            include_bytes!("../../pn-opus-fixture.py").as_slice(),
        ),
        (
            "fixed_opus_script_sha256",
            include_bytes!("../../opus-fixture.py").as_slice(),
        ),
        (
            "pcm_kernel_sha256",
            include_bytes!("../../pcm-kernel.mjs").as_slice(),
        ),
        (
            "pn_inspector_sha256",
            include_bytes!("../../pn-opus-markers.mjs").as_slice(),
        ),
    ] {
        if metadata["provenance"][key] != hash(bytes) {
            return Err(format!("frozen PN artifact differs: {key}").into());
        }
    }
    verify_markers(
        &decoded,
        &book,
        lookahead as u64,
        &metadata["marker_control"],
    )?;
    Ok(AudioArchive {
        metadata,
        sha256: hash(data),
        packets,
        kind: kind.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_books_and_finite_bounds_are_identical_for_both_roles() {
        let mic = book("mic", 20, "00000000-0000-0000-0000-000000000000").unwrap();
        let source = book("source", 20, "00000000-0000-0000-0000-000000000000").unwrap();
        assert_eq!(mic["shared_run"], source["shared_run"]);
        assert_eq!(mic["codebook_sha256"], source["codebook_sha256"]);
        assert_ne!(mic["markers"][0]["code"], source["markers"][0]["code"]);
        assert!(book("mic", 19, "00000000-0000-0000-0000-000000000000").is_err());
        assert!(book("mic", 361, "00000000-0000-0000-0000-000000000000").is_err());
    }
    #[test]
    fn replay_start_never_cuts_loops_or_trusts_requested_hold() {
        let archive = AudioArchive {
            metadata: json!({"schema":2,"duration_seconds":21}),
            sha256: String::new(),
            packets: Vec::new(),
            kind: "mic".into(),
        };
        assert_eq!(
            start_policy(&json!({"total_seconds":21}), &archive, false).unwrap(),
            (21, 0)
        );
        for hold in [0, 50, 200, 500] {
            assert_eq!(
                start_policy(
                    &json!({"total_seconds":21,"audio_hold_ms":hold}),
                    &archive,
                    true
                )
                .unwrap(),
                (21, hold)
            );
            assert!(
                start_policy(
                    &json!({"total_seconds":21,"audio_hold_ms":hold}),
                    &archive,
                    false
                )
                .is_err()
            );
        }
        for request in [
            json!({"seconds":20}),
            json!({"total_seconds":20}),
            json!({"total_seconds":22}),
            json!({"total_seconds":21,"seconds":21}),
            json!({"total_seconds":21,"audio_hold_ms":10}),
            json!({"total_seconds":21,"audio_hold_ms":"500"}),
            json!({"total_seconds":21,"loop":false}),
            json!({"total_seconds":21,"loops":1}),
            json!({"total_seconds":21,"rewind":false}),
            json!({"total_seconds":21,"endOrdinal":1008000}),
        ] {
            assert!(start_policy(&request, &archive, true).is_err());
        }
        let old = AudioArchive {
            metadata: json!({"schema":1}),
            ..archive
        };
        assert_eq!(
            start_policy(&json!({"seconds":120}), &old, false).unwrap(),
            (120, 0)
        );
        assert!(start_policy(&json!({"seconds":120,"audio_hold_ms":0}), &old, false).is_err());
    }
    #[test]
    fn pair_requires_complete_identical_books_not_only_matching_duration() {
        let mic = AudioArchive {
            metadata: json!({"schema":2,"duration_seconds":21,"pn":{"shared_run":{"run_id":"a"},"codebook_sha256":"a"}}),
            sha256: String::new(),
            packets: Vec::new(),
            kind: "mic".into(),
        };
        let mut source = AudioArchive {
            metadata: mic.metadata.clone(),
            sha256: String::new(),
            packets: Vec::new(),
            kind: "source".into(),
        };
        assert!(shared_pair(&mic, &source).is_ok());
        source.metadata["pn"]["shared_run"]["run_id"] = json!("b");
        assert!(shared_pair(&mic, &source).is_err());
        source.metadata = mic.metadata.clone();
        source.metadata["schema"] = json!(1);
        assert!(shared_pair(&mic, &source).is_err());
    }
    #[test]
    fn decoded_filter_rejects_missing_duplicate_and_wrong_source() {
        let b = book("mic", 20, "00000000-0000-0000-0000-000000000000").unwrap();
        let (_, mut waveform) = waveform("mic", 20, &b);
        let one = peaks(&waveform, 0, RATE);
        assert_eq!(one.len(), 10);
        assert!(peaks(&waveform, 64, RATE).is_empty());
        let first = waveform[RATE as usize..RATE as usize + LENGTH].to_vec();
        waveform[RATE as usize + 14400..RATE as usize + 14400 + LENGTH].copy_from_slice(&first);
        assert_eq!(peaks(&waveform, 0, RATE).len(), 11);
        assert!(peaks(&vec![0f32; waveform.len()], 0, RATE).is_empty());
    }
}
