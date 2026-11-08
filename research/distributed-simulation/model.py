"""Research-only, single-process ownership/fencing and transfer model.

This is NOT Minecraft code, a consensus implementation, or a proof of distributed safety.
A central logical ledger acts as an idealized quorum-certified decision oracle.
"""
from dataclasses import dataclass
from typing import Dict, FrozenSet, Tuple

Region = Tuple[str, int, int]


class Blocked(RuntimeError):
    """Fail-closed model operation."""


@dataclass(frozen=True)
class Grant:
    region: Region
    owner: str
    epoch: int
    supporters: FrozenSet[str]


@dataclass(frozen=True)
class Transfer:
    entity: str
    source: Region
    destination: Region
    source_epoch: int
    destination_epoch: int


class Oracle:
    """Idealized durable, linearizable authority record (NOT a distributed protocol)."""

    def __init__(self, members):
        self.members = frozenset(members)
        if not self.members:
            raise ValueError("membership cannot be empty")
        self._grants: Dict[Region, Grant] = {}

    def grant(self, region: Region, owner: str, supporters) -> Grant:
        votes = frozenset(supporters)
        if owner not in self.members or not votes <= self.members:
            raise Blocked("owner/voter not in canonical membership")
        if len(votes) < len(self.members) // 2 + 1:
            raise Blocked("no canonical majority")
        previous = self._grants.get(region)
        next_epoch = 1 if previous is None else previous.epoch + 1
        new_grant = Grant(region, owner, next_epoch, votes)
        self._grants[region] = new_grant
        return new_grant

    def verify(self, region: Region, owner: str, epoch: int):
        current = self._grants.get(region)
        if current is None or current.owner != owner or current.epoch != epoch:
            raise Blocked("fenced: non-current region owner/epoch")


class Simulation:
    """Atomically changes an entity's region only on explicit commit."""

    def __init__(self, oracle: Oracle):
        self.oracle = oracle
        self.entities: Dict[str, Region] = {}
        self.pending: Dict[str, Transfer] = {}
        self.completed: Dict[str, Transfer] = {}

    def spawn(self, entity: str, region: Region, owner: str, epoch: int):
        self.oracle.verify(region, owner, epoch)
        if entity in self.entities:
            raise Blocked("duplicate entity")
        self.entities[entity] = region

    def prepare(self, transfer_id: str, entity: str, destination: Region, source_owner: str,
                source_epoch: int, destination_owner: str, destination_epoch: int):
        if transfer_id in self.pending or transfer_id in self.completed:
            raise Blocked("duplicate transfer ID")
        source = self.entities.get(entity)
        if source is None:
            raise Blocked("entity missing")
        if any(t.entity == entity for t in self.pending.values()):
            raise Blocked("entity already in transit")
        if source == destination:
            raise Blocked("same-region transfer")
        self.oracle.verify(source, source_owner, source_epoch)
        self.oracle.verify(destination, destination_owner, destination_epoch)
        self.pending[transfer_id] = Transfer(entity, source, destination, source_epoch, destination_epoch)

    def commit(self, transfer_id: str, source_owner: str, destination_owner: str):
        # A retry after successful commit is idempotent in this idealized oracle.
        if transfer_id in self.completed:
            return
        transfer = self.pending.get(transfer_id)
        if transfer is None:
            raise Blocked("transfer was not prepared")
        self.oracle.verify(transfer.source, source_owner, transfer.source_epoch)
        self.oracle.verify(transfer.destination, destination_owner, transfer.destination_epoch)
        if self.entities.get(transfer.entity) != transfer.source:
            raise Blocked("source state changed")
        self.entities[transfer.entity] = transfer.destination
        self.completed[transfer_id] = transfer
        del self.pending[transfer_id]

    def abort(self, transfer_id: str):
        # A prepared transfer never materializes on destination without commit.
        self.pending.pop(transfer_id, None)
