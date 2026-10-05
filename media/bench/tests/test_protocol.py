import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location('protocol', Path(__file__).resolve().parents[1] / 'protocol-diagnostics.py')
protocol = importlib.util.module_from_spec(spec)
spec.loader.exec_module(protocol)


class ProtocolTests(unittest.TestCase):
    def test_compound_report_and_remb_target_sources(self):
        report = struct.pack('!BBHII', 0x81, 201, 7, 99, 123) + bytes([0, 0, 0, 0]) + struct.pack('!IIII', 42, 7, 0, 0)
        remb = struct.pack('!BBHII', 0x8f, 206, 5, 99, 0) + b'REMB' + bytes([1, 0, 3, 232]) + struct.pack('!I', 123)
        values = list(protocol.rtcp(report + remb))
        self.assertEqual(values[0]['reports'][0]['target_ssrc'], 123)
        self.assertEqual(values[1]['remb'], {'bitrate_bps': 1000, 'target_ssrcs': [123]})

    def test_twcc_signed_large_delta_and_received_packet_count(self):
        # Three statuses [small delta, missing, large delta] in a two-bit vector.
        header = struct.pack('!BBHIIHH', 0x8f, 205, 6, 99, 123, 500, 3)
        packet = header + bytes([0, 0, 1, 2]) + struct.pack('!H', 0xd200) + bytes([4]) + struct.pack('!h', -2) + b'\x00\x00\x00'
        value = list(protocol.rtcp(packet))[0]['twcc']
        self.assertEqual(value['received_count'], 2)
        self.assertEqual(value['arrival_delta_us_min'], -500)
        self.assertEqual(value['arrival_delta_us_sum'], 500)

    def test_all_batched_packet_blobs_and_truncation(self):
        self.assertEqual(list(protocol.blobs(b'\x02\x03aabbb', 2)), [b'aa', b'bbb'])
        with self.assertRaises(ValueError): list(protocol.blobs(b'\x04aa', 1))
        with self.assertRaises(ValueError): list(protocol.rtcp(b'\x80\xc9\x00\x04'))

    def test_packet_delay_is_normalized_over_the_measurement_interval(self):
        before = {'id': 'v', 'type': 'outbound-rtp', 'kind': 'video', 'packetsSent': 1000, 'totalPacketSendDelay': 50, 'framesEncoded': 60}
        after = {**before, 'packetsSent': 2000, 'totalPacketSendDelay': 55, 'framesEncoded': 660}
        result = protocol.browser_diagnostics({'samples': [{'at': 1000, 'stats': [before]}, {'at': 11000, 'stats': [after]}]})
        self.assertEqual(result['video'][0]['mean_packet_send_delay_ms'], 5)
        self.assertEqual(result['video'][0]['framesEncoded_per_second'], 60)


if __name__ == '__main__': unittest.main()
