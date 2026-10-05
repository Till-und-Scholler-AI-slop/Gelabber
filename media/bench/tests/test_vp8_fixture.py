import importlib.util
from pathlib import Path
import struct
import tomllib
import unittest

spec = importlib.util.spec_from_file_location("vp8_fixture", Path(__file__).parents[1] / "vp8-fixture.py")
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


def frame(key=False, length=20):
    prefix = b"\x10\0\0\x9d\x01\x2a" + struct.pack("<HH", 1920, 1080) if key else b"\x11\0\0"
    return prefix + b"\x42" * (length - len(prefix))


def ivf(frames, timestamps=None, count=None):
    timestamps = timestamps or list(range(len(frames)))
    header = struct.pack("<4sHH4sHHIIII", b"DKIF", 0, 32, b"VP80", 1920, 1080, 60, 1, count if count is not None else len(frames), 0)
    return header + b"".join(struct.pack("<IQ", len(data), timestamp) + data for data, timestamp in zip(frames, timestamps))


class FixtureTests(unittest.TestCase):
    def test_native_source_preserves_shared_locked_dependency_versions(self):
        base = tomllib.loads((Path(__file__).parents[1] / 'current-probe/Cargo.lock').read_text())['package']
        replay = tomllib.loads((Path(__file__).parents[1] / 'rtp-source/Cargo.lock').read_text())['package']
        known = {(package['name'], package['version']) for package in base}
        changed = {(package['name'], package['version']) for package in replay} - known
        self.assertEqual(changed, {('gelabber-fixed-rtp-source', '0.1.0')})

    def test_exact_fps_and_initial_keyframe_required(self):
        frames = [frame(True)] + [frame()] * 59
        self.assertEqual(fixture.parse_ivf(ivf(frames)), frames)
        with self.assertRaises(ValueError): fixture.parse_ivf(ivf(frames[1:] + [frame()]))
        with self.assertRaises(ValueError): fixture.parse_ivf(ivf(frames[:-1]))

    def test_truncated_miscount_and_missing_timestamp_are_rejected(self):
        frames = [frame(True)] + [frame()] * 59
        for data in [ivf(frames)[:-1], ivf(frames, count=61), ivf(frames, list(range(1, 61)))]:
            with self.assertRaises(ValueError): fixture.parse_ivf(data)

    def test_keyframe_dimension_and_policy_are_verified(self):
        frames = [frame(True)] + [frame()] * 179
        with self.assertRaises(ValueError): fixture.parse_ivf(ivf(frames))
        bad = bytearray(frame(True)); bad[6:8] = struct.pack("<H", 1280)
        with self.assertRaises(ValueError): fixture.parse_ivf(ivf([bytes(bad)] + [frame()] * 59))

    def test_fragment_reassembly_markers_picture_ids_and_schedule(self):
        frames = [frame(True, 51_594), frame(False, 3030), frame(False, 2000)]
        output = list(fixture.packets(frames))
        rebuilt, payloads, last_timestamp, last_due, last_sequence = [], [], None, -1, 0x1fff
        for due, packet in output:
            version, marker_pt, sequence, timestamp, ssrc = struct.unpack_from("!BBHII", packet)
            self.assertEqual((version, marker_pt & 0x7f, ssrc), (0x80, 96, fixture.SSRC))
            self.assertEqual(sequence, (last_sequence + 1) & 0xffff)
            self.assertLessEqual(len(packet), fixture.MTU)
            self.assertGreater(due, last_due)
            if timestamp != last_timestamp:
                self.assertTrue(packet[12] & 0x10)
                self.assertEqual(timestamp, len(rebuilt) * 1500)
                self.assertEqual(((packet[14] & 0x7f) << 8) | packet[15], len(rebuilt))
                payloads = []
            else: self.assertFalse(packet[12] & 0x10)
            self.assertEqual(packet[12] & 0x0f, 0)
            payloads.append(packet[16:])
            self.assertGreaterEqual(due, timestamp // 1500 * 1_000_000_000 // 60)
            self.assertLess(due, (timestamp // 1500 + 1) * 1_000_000_000 // 60)
            if marker_pt & 0x80: rebuilt.append(b"".join(payloads))
            last_due, last_sequence, last_timestamp = due, sequence, timestamp
        self.assertEqual(rebuilt, frames)

    def test_sequence_and_picture_id_wrap_without_payload_changes(self):
        frames = [frame(True)] * 70_000
        output = list(fixture.packets(frames))
        packet = output[-1][1]
        self.assertEqual(struct.unpack_from("!H", packet, 2)[0], (0x2000 + 69_999) & 0xffff)
        self.assertEqual(((packet[14] & 0x7f) << 8) | packet[15], 69_999 & 0x7fff)
        self.assertEqual(packet[16:], frames[-1])


if __name__ == "__main__": unittest.main()
