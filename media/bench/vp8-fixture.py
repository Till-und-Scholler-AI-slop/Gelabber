#!/usr/bin/env python3
"""Build/inspect one frozen VP8 source and its RFC 7741 RTP replay archive.

This is an offline instrument. It neither starts a server nor sends packets.
The output is a fixed VBR frame schedule, not a promise of per-packet CBR.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess

MAGIC = b"GVP8RTP1"
WIDTH, HEIGHT, FPS, TARGET = 1920, 1080, 60, 4_000_000
MTU = 1200  # RTP datagram size before DTLS/SRTP/IP overhead.
SSRC = 0x47565038


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command(args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def parse_ivf(data):
    if len(data) < 32:
        raise ValueError("truncated IVF header")
    signature, version, header, codec, width, height, rate, scale, count, reserved = struct.unpack_from("<4sHH4sHHIIII", data)
    if (signature, version, header, codec, width, height, rate, scale, reserved) != (b"DKIF", 0, 32, b"VP80", WIDTH, HEIGHT, FPS, 1, 0):
        raise ValueError("requires IVF VP8 1920x1080 with the exact 60/1 timebase")
    frames, offset = [], 32
    while offset < len(data):
        if offset + 12 > len(data):
            raise ValueError("truncated IVF frame header")
        length, timestamp = struct.unpack_from("<IQ", data, offset)
        offset += 12
        if length < 3 or offset + length > len(data):
            raise ValueError("truncated/empty VP8 frame")
        payload = data[offset:offset + length]
        if timestamp != len(frames):
            raise ValueError("non-contiguous frame timestamps or non-zero initial timestamp")
        keyframe = not payload[0] & 1
        if keyframe:
            if len(payload) < 10 or payload[3:6] != b"\x9d\x01\x2a":
                raise ValueError("invalid VP8 keyframe header")
            dimensions = struct.unpack_from("<HH", payload, 6)
            if tuple(value & 0x3fff for value in dimensions) != (WIDTH, HEIGHT):
                raise ValueError("keyframe resolution differs from the IVF header")
        frames.append(payload)
        offset += length
    if not frames or len(frames) != count or frames[0][0] & 1 or count % FPS:
        raise ValueError("needs whole seconds, an initial keyframe and exact IVF frame count")
    # Each loop starts independently; keyframe gaps must not exceed the declared 2s policy.
    keys = [i for i, frame in enumerate(frames) if not frame[0] & 1]
    if max(b - a for a, b in zip(keys, keys[1:] + [count])) > 2 * FPS:
        raise ValueError("keyframe interval exceeds two seconds")
    return frames


def packets(frames, mtu=MTU):
    """Deterministic VP8 payloads with a 15-bit PictureID, constant 90kHz clock.

    A frame may span multiple partitions without splitting on their boundaries
    (RFC 7741 §4.4). S=1 only on its first packet; PartID=0 throughout.
    Packet fragments are spread evenly inside each 1/60-second frame interval.
    """
    capacity, sequence = mtu - 12 - 4, 0x2000
    if capacity < 10:
        raise ValueError("RTP MTU is too small")
    for index, frame in enumerate(frames):
        count = math.ceil(len(frame) / capacity)
        for fragment in range(count):
            chunk = frame[fragment * capacity:(fragment + 1) * capacity]
            descriptor = bytes([0x90 if fragment == 0 else 0x80, 0x80, 0x80 | ((index >> 8) & 0x7f), index & 0xff])
            header = struct.pack("!BBHII", 0x80, 96 | (0x80 if fragment == count - 1 else 0), sequence & 0xffff, (index * 1500) & 0xffffffff, SSRC)
            due_ns = ((index * count + fragment) * 1_000_000_000) // (FPS * count)
            yield due_ns, header + descriptor + chunk
            sequence += 1


def describe(path, frames):
    sizes = [len(frame) for frame in frames]
    duration = len(frames) / FPS
    # No filler/padding: every encoded byte belongs to the clip.
    rtp = list(packets(frames))
    return {
        "schema": 1, "source": "ffmpeg-testsrc2-1080p60-v1", "codec": "VP8",
        "width": WIDTH, "height": HEIGHT, "fps": FPS, "frames": len(frames),
        "duration_seconds": duration, "target_payload_bitrate_bps": TARGET,
        "ivf_sha256": sha256(path), "encoded_bytes": sum(sizes),
        "encoded_bitrate_bps": sum(sizes) * 8 / duration,
        "frame_bytes_min": min(sizes), "frame_bytes_max": max(sizes),
        "keyframe_indices": [i for i, frame in enumerate(frames) if not frame[0] & 1],
        "rtp_packets": len(rtp), "rtp_payload_bytes": sum(len(packet) - 12 for _, packet in rtp),
        "rtp_payload_bitrate_bps": sum(len(packet) - 12 for _, packet in rtp) * 8 / duration,
        "rtp_datagram_bitrate_bps": sum(len(packet) for _, packet in rtp) * 8 / duration,
        "rtp_mtu": MTU, "rtp_clock_hz": 90_000, "picture_id_bits": 15,
        "schedule": "CFR 60fps; fragments evenly spaced within their frame interval",
        "congestion_adaptation": False, "padding_bytes": 0,
        "comparison_available": False, "production_feature_acceptance": False,
    }


def decode_check(path):
    args = ["ffprobe", "-v", "error", "-count_frames", "-show_entries", "stream=codec_name,width,height,r_frame_rate,nb_read_frames", "-of", "json", str(path)]
    result = subprocess.run(args, check=True, capture_output=True, text=True)
    if result.stderr.strip():
        raise ValueError("decoder emitted errors: " + result.stderr.strip())
    return json.loads(result.stdout)["streams"], {"command": args, "stderr": result.stderr}


def binary_provenance(name):
    path = Path(shutil.which(name) or name).resolve(strict=True)
    libraries = []
    for line in command(["ldd", str(path)]).splitlines():
        # Capture the codec library actually loaded by this FFmpeg binary.
        if "libvpx" in line and "=>" in line:
            library = Path(line.split("=>", 1)[1].split()[0]).resolve(strict=True)
            libraries.append({"path": str(library), "sha256": sha256(library)})
    return {"path": str(path), "sha256": sha256(path), "version": command([str(path), "-version"]).splitlines()[0], "libvpx": libraries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ivf", type=Path)
    parser.add_argument("--build", action="store_true", help="encode a fresh file with the installed FFmpeg; never overwrite")
    parser.add_argument("--seconds", type=int, default=10)
    parser.add_argument("--rtp", type=Path, help="write a fresh replay archive; never overwrite")
    parser.add_argument("--output", type=Path, required=True, help="fresh JSON evidence file")
    args = parser.parse_args()
    if args.seconds < 2 or args.seconds > 120 or args.output.exists() or (args.rtp and args.rtp.exists()):
        parser.error("seconds must be 2..120 and all output files must be fresh")
    encoding = None
    if args.build:
        if args.ivf.exists():
            parser.error("IVF already exists")
        encoding = ["ffmpeg", "-nostdin", "-n", "-v", "warning", "-f", "lavfi", "-i", f"testsrc2=size={WIDTH}x{HEIGHT}:rate={FPS}", "-t", str(args.seconds), "-an", "-c:v", "libvpx", "-deadline", "realtime", "-cpu-used", "8", "-threads", "1", "-b:v", str(TARGET), "-minrate", str(TARGET), "-maxrate", str(TARGET), "-bufsize", str(2 * TARGET), "-g", str(2 * FPS), "-lag-in-frames", "0", "-auto-alt-ref", "0", "-drop-threshold", "0", "-undershoot-pct", "5", "-overshoot-pct", "5", "-f", "ivf", str(args.ivf)]
        command(encoding)
    frames = parse_ivf(args.ivf.read_bytes())
    evidence = describe(args.ivf, frames)
    streams, decoder = decode_check(args.ivf)
    evidence.update({"ffmpeg": binary_provenance("ffmpeg"), "ffprobe": binary_provenance("ffprobe"), "encoding_command": encoding, "decoded_streams": streams, "decoder_validation": decoder})
    expected = {"codec_name": "vp8", "width": WIDTH, "height": HEIGHT, "r_frame_rate": "60/1", "nb_read_frames": str(len(frames))}
    if evidence["decoded_streams"] != [expected]:
        raise ValueError("complete decoder check does not match the frozen frame stream")
    if not .9 * TARGET <= evidence["encoded_bitrate_bps"] <= 1.1 * TARGET:
        raise ValueError("actual encoded source bitrate is outside 4Mbit ±10%")
    if args.rtp:
        header = json.dumps(evidence, sort_keys=True, separators=(",", ":")).encode()
        with args.rtp.open("xb") as output:
            output.write(MAGIC + struct.pack("<I", len(header)) + header)
            for due_ns, packet in packets(frames):
                output.write(struct.pack("<QI", due_ns, len(packet)) + packet)
        evidence["rtp_archive_sha256"] = sha256(args.rtp)
    with args.output.open("x") as output:
        json.dump(evidence, output, indent=2)
        output.write("\n")
    print(json.dumps({key: evidence[key] for key in ["frames", "encoded_bitrate_bps", "rtp_payload_bitrate_bps", "ivf_sha256"]}))


if __name__ == "__main__":
    main()
