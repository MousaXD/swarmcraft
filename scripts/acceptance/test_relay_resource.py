"""Contract checks for the local shared-netem evidence gate."""
from __future__ import annotations

import unittest
from unittest.mock import patch

import relay_resource as gate

RECEIPT = """
running 1 test
test private_relay_expires_staggered_active_circuits_without_cross_disconnect ... relay_duration bulk acknowledged_bytes=1392640 samples=170 overlap_samples=112 dial_to_close_ms=20120 authenticated_to_close_ms=19818 payload_kib_per_s=68.62 p95_echo_ms=91.099
relay_duration interactive acknowledged_bytes=9088 samples=142 overlap_samples=90 dial_to_close_ms=20247 authenticated_to_close_ms=19718 payload_kib_per_s=0.45 p95_echo_ms=82.226
relay_duration configured_seconds=20 byte_budget=67108864 overlap_ms=13031 independent_post_expiry_samples=52 authentication_revoked_both_pairs=true
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 27.04s
"""


class ReceiptTests(unittest.TestCase):
    def test_actual_receipt(self):
        result = gate.parse_receipt(RECEIPT)
        self.assertEqual(result['bulk']['acknowledged_bytes'], 1392640)
        self.assertEqual(result['interactive']['samples'], 142)
        self.assertEqual(result['summary']['independent_post_expiry_samples'], 52)
        self.assertIs(result['summary']['authentication_revoked_both_pairs'], True)

    def test_skipped_failed_or_duplicate_run_refused(self):
        for text in (RECEIPT.replace('1 passed', '0 passed'),
                     RECEIPT.replace('0 failed', '1 failed'),
                     RECEIPT.replace('0 ignored', '1 ignored'), RECEIPT + RECEIPT):
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                gate.parse_receipt(text)

    def test_missing_or_inconsistent_payload_and_overlap_refused(self):
        for old, new in (('1392640', '1392641'), ('overlap_samples=112', 'overlap_samples=9'),
                         ('overlap_samples=112', 'overlap_samples=171'), ('dial_to_close_ms=20120', 'dial_to_close_ms=1000'),
                         ('authenticated_to_close_ms=19818', 'authenticated_to_close_ms=30000'),
                         ('independent_post_expiry_samples=52', 'independent_post_expiry_samples=4'),
                         ('authentication_revoked_both_pairs=true', 'authentication_revoked_both_pairs=false'),
                         ('configured_seconds=20', 'configured_seconds=30'), ('byte_budget=67108864', 'byte_budget=2')):
            with self.subTest(old=old), self.assertRaises(RuntimeError):
                gate.parse_receipt(RECEIPT.replace(old, new))


class KernelContractTests(unittest.TestCase):
    def test_kernel_rate_and_delay_are_verified(self):
        row = {'kind': 'netem', 'root': True, 'options': {'rate': {'rate': 500000}, 'delay': {'delay': 0.005}}}
        with patch.object(gate, 'qdiscs', return_value=[row]):
            self.assertEqual(gate.netem(4, 5), row)
            for rate, delay in ((1, 5), (4, 15)):
                with self.assertRaises(RuntimeError):
                    gate.netem(rate, delay)
        for rows in ([], [row, row], [{'kind': 'noqueue'}], [{**row, 'root': False}]):
            with patch.object(gate, 'qdiscs', return_value=rows), self.assertRaises(RuntimeError):
                gate.netem(4, 5)


if __name__ == '__main__':
    unittest.main()
