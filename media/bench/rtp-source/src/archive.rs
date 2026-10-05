//! Strict replay-archive reader. No encoder, padding, frame skipping or BWE.
use bytes::{Bytes, BytesMut};
use rtc::{rtp, shared::marshal::Unmarshal};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{error::Error, time::Duration};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
pub const SSRC: u32 = 0x47565038;
pub const FPS: u64 = 60;

pub fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

#[derive(Clone)]
pub struct Record {
    pub due: Duration,
    pub packet: rtp::Packet,
}

pub struct Archive {
    pub metadata: Value,
    pub sha256: String,
    pub period: Duration,
    pub frames: u64,
    pub records: Vec<Record>,
}

fn take<'a>(data: &mut &'a [u8], count: usize) -> Result<&'a [u8]> {
    if data.len() < count {
        return Err("truncated replay archive".into());
    }
    let (part, rest) = data.split_at(count);
    *data = rest;
    Ok(part)
}

impl Archive {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let mut data = raw;
        if take(&mut data, 8)? != b"GVP8RTP1" {
            return Err("incorrect replay magic".into());
        }
        let size = u32::from_le_bytes(take(&mut data, 4)?.try_into()?) as usize;
        if size > 1_048_576 {
            return Err("oversized replay metadata".into());
        }
        let metadata: Value = serde_json::from_slice(take(&mut data, size)?)?;
        let frames = metadata["frames"].as_u64().ok_or("missing frame count")?;
        if metadata["schema"] != 1
            || metadata["codec"] != "VP8"
            || metadata["fps"] != FPS
            || metadata["width"] != 1920
            || metadata["height"] != 1080
            || metadata["rtp_mtu"] != 1200
            || metadata["padding_bytes"] != 0
            || metadata["congestion_adaptation"] != false
            || !(120..=7200).contains(&frames)
            || frames % FPS != 0
            || metadata["duration_seconds"].as_f64() != Some(frames as f64 / FPS as f64)
        {
            return Err("unsupported replay source policy".into());
        }
        let mut records = Vec::new();
        while !data.is_empty() {
            let due_ns = u64::from_le_bytes(take(&mut data, 8)?.try_into()?);
            let size = u32::from_le_bytes(take(&mut data, 4)?.try_into()?) as usize;
            if !(17..=1200).contains(&size) {
                return Err("invalid RTP datagram length".into());
            }
            let packet = rtp::Packet::unmarshal(&mut BytesMut::from(take(&mut data, size)?))?;
            records.push(Record {
                due: Duration::from_nanos(due_ns),
                packet,
            });
        }
        let mut ivf = Vec::new();
        ivf.extend(b"DKIF\x00\x00\x20\x00VP80");
        ivf.extend(1920u16.to_le_bytes());
        ivf.extend(1080u16.to_le_bytes());
        ivf.extend(60u32.to_le_bytes());
        ivf.extend(1u32.to_le_bytes());
        ivf.extend((frames as u32).to_le_bytes());
        ivf.extend(0u32.to_le_bytes());
        let mut index = 0usize;
        let mut total_payload = 0u64;
        let mut total_encoded = 0u64;
        for frame in 0..frames {
            let first = index;
            let mut encoded: Vec<u8> = Vec::new();
            loop {
                let record = records.get(index).ok_or("missing frame packets")?;
                let packet = &record.packet;
                let header = &packet.header;
                let payload = &packet.payload;
                let descriptor = [
                    if index == first { 0x90 } else { 0x80 },
                    0x80,
                    0x80 | ((frame >> 8) as u8 & 0x7f),
                    frame as u8,
                ];
                if header.version != 2
                    || header.extension
                    || header.padding
                    || !header.csrc.is_empty()
                    || header.payload_type != 96
                    || header.ssrc != SSRC
                    || header.sequence_number != 0x2000u16.wrapping_add(index as u16)
                    || header.timestamp != (frame * 1500) as u32
                    || payload.len() < 5
                    || payload[..4] != descriptor
                {
                    return Err("RTP sequence/clock/descriptor differs from frozen policy".into());
                }
                encoded.extend(&payload[4..]);
                total_payload += payload.len() as u64;
                index += 1;
                if header.marker {
                    break;
                }
            }
            let count = index - first;
            for fragment in 0..count {
                let expected_ns = ((frame * count as u64 + fragment as u64) * 1_000_000_000)
                    / (FPS * count as u64);
                if records[first + fragment].due.as_nanos() != expected_ns as u128 {
                    return Err("packet schedule differs from frozen CFR policy".into());
                }
            }
            if frame == 0
                && (encoded.len() < 10 || encoded[0] & 1 != 0 || encoded[3..6] != [0x9d, 1, 0x2a])
            {
                return Err("replay must begin with a VP8 keyframe".into());
            }
            total_encoded += encoded.len() as u64;
            ivf.extend((encoded.len() as u32).to_le_bytes());
            ivf.extend(frame.to_le_bytes());
            ivf.extend(encoded);
        }
        let seconds = frames / FPS;
        let bitrate = total_encoded as f64 * 8.0 / seconds as f64;
        if index != records.len()
            || metadata["rtp_packets"] != index
            || metadata["encoded_bytes"] != total_encoded
            || metadata["rtp_payload_bytes"] != total_payload
            || metadata["ivf_sha256"] != hash(&ivf)
            || !(3_600_000.0..=4_400_000.0).contains(&bitrate)
        {
            return Err(
                "reconstructed clip/hash/actual bitrate differs from source evidence".into(),
            );
        }
        Ok(Self {
            metadata,
            sha256: hash(raw),
            period: Duration::from_secs(seconds),
            frames,
            records,
        })
    }

    pub fn packet(&self, index: usize, cycle: u64) -> rtp::Packet {
        let mut packet = self.records[index].packet.clone();
        packet.header.sequence_number =
            0x2000u16.wrapping_add((cycle * self.records.len() as u64 + index as u64) as u16);
        packet.header.timestamp = packet
            .header
            .timestamp
            .wrapping_add((cycle * self.frames * 1500) as u32);
        let frame = self.records[index].packet.header.timestamp as u64 / 1500 + cycle * self.frames;
        let mut payload = packet.payload.to_vec();
        payload[2] = 0x80 | ((frame >> 8) as u8 & 0x7f);
        payload[3] = frame as u8;
        packet.payload = Bytes::from(payload);
        packet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc::shared::marshal::Marshal;

    // Transport parser fixture; synthetic payload bytes do not claim codec validation.
    fn synthetic_archive() -> Vec<u8> {
        let mut ivf = b"DKIF\x00\x00\x20\x00VP80".to_vec();
        ivf.extend(1920u16.to_le_bytes());
        ivf.extend(1080u16.to_le_bytes());
        ivf.extend(60u32.to_le_bytes());
        ivf.extend(1u32.to_le_bytes());
        ivf.extend(120u32.to_le_bytes());
        ivf.extend(0u32.to_le_bytes());
        let mut records = Vec::new();
        for frame in 0..120u64 {
            let mut encoded = vec![0x42; 8333];
            encoded[0] = 0x11;
            if frame == 0 {
                encoded[..10].copy_from_slice(&[0x10, 0, 0, 0x9d, 1, 0x2a, 0x80, 0x07, 0x38, 0x04]);
            }
            ivf.extend(8333u32.to_le_bytes());
            ivf.extend(frame.to_le_bytes());
            ivf.extend(&encoded);
            for (fragment, chunk) in encoded.chunks(1184).enumerate() {
                let mut payload = vec![
                    if fragment == 0 { 0x90 } else { 0x80 },
                    0x80,
                    0x80,
                    frame as u8,
                ];
                payload.extend(chunk);
                let packet = rtp::Packet {
                    header: rtp::Header {
                        version: 2,
                        payload_type: 96,
                        ssrc: SSRC,
                        sequence_number: 0x2000 + frame as u16 * 8 + fragment as u16,
                        timestamp: frame as u32 * 1500,
                        marker: fragment == 7,
                        ..Default::default()
                    },
                    payload: Bytes::from(payload),
                };
                records.push((
                    ((frame * 8 + fragment as u64) * 1_000_000_000) / 480,
                    packet.marshal().unwrap(),
                ));
            }
        }
        let metadata = serde_json::json!({"schema":1,"codec":"VP8","fps":60,"width":1920,"height":1080,"rtp_mtu":1200,"padding_bytes":0,"congestion_adaptation":false,"frames":120,"duration_seconds":2.0,"rtp_packets":960,"encoded_bytes":999960,"rtp_payload_bytes":1003800,"ivf_sha256":hash(&ivf)});
        let header = serde_json::to_vec(&metadata).unwrap();
        let mut raw = b"GVP8RTP1".to_vec();
        raw.extend((header.len() as u32).to_le_bytes());
        raw.extend(header);
        for (due, packet) in records {
            raw.extend(due.to_le_bytes());
            raw.extend((packet.len() as u32).to_le_bytes());
            raw.extend(packet);
        }
        raw
    }

    #[test]
    fn source_hash_and_schedule_prevent_silent_workload_changes() {
        let raw = synthetic_archive();
        let archive = Archive::parse(&raw).unwrap();
        assert_eq!(archive.frames, 120);
        assert_eq!(archive.records.len(), 960);
        let header_size = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
        let mut late = raw.clone();
        late[12 + header_size] = 1;
        assert!(Archive::parse(&late).is_err());
        let mut changed = raw.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert!(Archive::parse(&changed).is_err());
        assert!(Archive::parse(&raw[..raw.len() - 1]).is_err());
        let mut extra = raw.clone();
        extra.extend(&raw[12 + header_size..]);
        assert!(Archive::parse(&extra).is_err());
    }

    #[test]
    fn truncation_is_a_failure_instead_of_a_partial_workload() {
        for raw in [
            vec![],
            b"GVP8RTP1".to_vec(),
            b"GVP8RTP1\xff\xff\xff\xff".to_vec(),
        ] {
            assert!(Archive::parse(&raw).is_err());
        }
    }
    #[test]
    fn loops_have_continuous_rtp_clocks_and_picture_ids() {
        let archive = Archive {
            metadata: Value::Null,
            sha256: String::new(),
            period: Duration::from_secs(10),
            frames: 600,
            records: vec![Record {
                due: Duration::ZERO,
                packet: rtp::Packet {
                    header: rtp::Header {
                        timestamp: 0,
                        ..Default::default()
                    },
                    payload: Bytes::from_static(&[0x90, 0x80, 0x80, 0, 0x42]),
                },
            }],
        };
        let packet = archive.packet(0, 120);
        assert_eq!(packet.header.timestamp, 108_000_000);
        assert_eq!(packet.header.sequence_number, 0x2000 + 120);
        assert_eq!(
            ((packet.payload[2] as u16 & 0x7f) << 8) | packet.payload[3] as u16,
            (72_000u32 & 0x7fff) as u16
        );
        assert_eq!(packet.payload[4], 0x42);
    }
}
