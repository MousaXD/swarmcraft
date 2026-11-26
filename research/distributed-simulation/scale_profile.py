"""Exclusive monotonic wall-clock phases for the #63 lab (not attestation)."""
from contextlib import contextmanager
import time

from region_runtime import require

PHASES = (
    "client_input_network", "command_journal", "seal_inputs",
    "runtime_preflight", "worker_tick", "worker_receipts", "resource_guard",
    "publication_receipt_validation", "independent_candidate_validation",
    "independent_serial_replay", "durable_client_receipts",
    "checkpoint_serialization", "canonical_journal_write",
    "ledger_read", "state_decode_roots", "sqlite_begin",
    "sqlite_durable_commit", "sqlite_read_commit",
    "client_receipt_lookup", "client_receipt_network", "client_acknowledgment",
    "other",
)


class PhaseClock:
    """Nested scopes attribute elapsed time exclusively, never double-count it."""

    def __init__(self):
        self.values = None
        self.stack = []

    def begin(self):
        require(self.values is None and not self.stack, "overlapping profile ticks")
        self.values = dict.fromkeys(PHASES, 0)

    @contextmanager
    def phase(self, name):
        if self.values is None:
            yield
            return
        require(name in self.values and name != "other", "unknown phase")
        start = time.monotonic_ns()
        self.stack.append(0)
        try:
            yield
        finally:
            duration = time.monotonic_ns() - start
            children = self.stack.pop()
            require(duration >= children, "invalid nested timing")
            self.values[name] += duration - children
            if self.stack:
                self.stack[-1] += duration

    def finish(self, elapsed_ns):
        require(self.values is not None and not self.stack, "unfinished profile scope")
        remainder = elapsed_ns - sum(self.values.values())
        require(remainder >= 0, "phase durations exceed measured service time")
        self.values["other"] = remainder
        values, self.values = self.values, None
        return values
