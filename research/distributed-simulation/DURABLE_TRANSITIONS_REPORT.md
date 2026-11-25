# #61 — Durable reference player, entity and inventory transitions

Date: 25 November 2026 UTC / 26 November UAE time.

**Locally verified research only.** This extends the #60 trusted
single-ledger reference authority. It is not a production Minecraft
runtime, distributed consensus, or independent-host certification.

## Supported ruleset and durable command contract

The new lab protocol 2 and transaction-v2 ruleset have a distinct
SQLite application ID and world-cut digest domain. The #60 legacy
authority refuses the transaction database, and vice versa. There is
no automatic migration or compatibility claim for Minecraft saves.

Each authenticated reference client can submit at most one command
per next tick. The command binds a unique ID, world identity, exact
parent cut, tick, direction and inventory action. Player identity is
provided by the lab's per-connection authentication rather than the
message. A new request is durably recorded before it can be sealed
for execution. Reusing an ID with a different player or payload, using
a new ID for the same player/tick, or targeting a stale/future cut is
refused. An exact pending replay returns its queued status; an exact
committed replay returns the original receipt, not a newly executed
action or current view.

A sealed batch includes every fixed player, with no-op movement and
inventory action for players without inputs. The batch cannot change
after sealing, including after a restart or generation rotation. A
registered player's explicit pickup/drop transfers at most one token
between their inventory and the occupied source cell, before
one-cell movement. An empty bank, empty inventory or full destination
prevents the debit; contention is resolved in ascending agent-ID
order. Uncontrolled background entities retain the prior autonomous
rules. Existing neighbor-signal propagation, periodic resource
transfer and global tick semantics remain deterministic. These are
abstract cell-bank interactions, not Minecraft blocks, redstone,
crafting, portals or NBT inventories.

The coordinator routes commands to the resident region and stages the
existing boundary transfers. Inside the #60 durable authority write
transaction, all current-generation authenticated region receipts must
agree with the exact sealed command batch and independent monolithic
serial simulation. The complete world, tick history, sealed-batch
result, and all committed player receipts are published atomically.
Pending/queued commands are not committed effects. A reader cannot
observe a committed command receipt for a cut that has not committed.

Reopening the database checks the journal from genesis, replays the
sealed batches, validates the accepted history and every stored
receipt, and rejects an inconsistent head. Pending commands and their
sealed batch remain durable. The trusted controller must obtain a
fresh generation before restarting workers. Exact command retries can
recover a committed receipt even after later ticks. The full replay
and per-tick independent reference validation are intentionally
expensive research checks; no scalability claim follows.

## Retained live evidence

- [Run 1](evidence/2026-11-26-transition-1.json)
- [Run 2](evidence/2026-11-26-transition-2.json)
- [Legacy static network regression](evidence/2026-11-26-transition-static-regression.json)

Each run used the same 256-agent fixture and three fixed registered
player identities, with two active reference clients. Six worker
processes were launched across three successive generations, with two
workers in distinct private network namespaces at a time. Client
processes used real authenticated loopback TCP in the private
coordinator namespace. Each connection had an ephemeral lab key;
no production identity or signing material was used.

The two active players started at opposite worker boundaries. Both
picked up a token and crossed on tick 1. Both dropped a token and
crossed back on tick 2, picked up and crossed again on tick 3, and
dropped and crossed back on tick 4. Their inventories followed
5 -> 6 -> 5 -> 6 -> 5 exactly, without a duplicate or missing agent
or resource.

| Verification | Run 1 | Run 2 |
| --- | --- | --- |
| Durable accepted ticks | 4 | 4 |
| Unique committed player commands | 8 | 8 |
| Player crossings between workers | 8 | 8 |
| Real TCP client sessions | 14 | 14 |
| Authority-writer SIGKILL faults | 2 | 2 |
| Lost-ack committed receipts recovered | 2 | 2 |
| Conflicting/spoofed replays refused | 2 | 2 |

In the pre-commit scenario, the isolated SQLite authority writer was
SIGKILLed after the transaction had staged command results, batch
result, world state and history but before COMMIT. Neither client
received an acknowledgment. Tick 1 remained canonical; the exact
sealed tick-2 inputs remained pending. After reopening the ledger,
rotating the generation and starting fresh workers, both clients
reconnected with the exact requests and tick 2 committed once.

In the post-commit scenario the writer was SIGKILLed after SQLite
COMMIT returned but before an acknowledgment. Both client connections
closed without replies. Reopening recovered tick 3 and its receipts;
both client retries returned the original results without advancing
the tick or applying the inventory operation again. Changed payload
and cross-player ID replay were refused. A normal tick 4 then
committed with the third generation.

Both final simulation roots were
`c808abefb44fedcfea85fadadba4784409f60c971e63192d482a054b4f3d4c80`.
World IDs, command IDs and session IDs differ across runs. The
independent evidence verifier reconstructs all eight regions, client
views, exact input batches and expected receipts from the serial
reference. It checks pending/committed journal boundaries, generation
rotation and the recorded process-kill outcomes. It does not attest
process execution or authenticate private MACs offline.

## Local validation

71 research tests passed (13 new). The added tests cover exact
pending and committed replay, replay after later ticks, invalid
identity/cut/payload and duplicate IDs, immutable sealed input,
unsealed or wrong candidates despite current worker receipts, legacy
schema refusal, atomic reader visibility, corrupt journal reopen,
action bounds, contested pickup of the last token, empty inventory,
full bank and cross-boundary signal/resource flow. Sixteen deliberate
alterations of retained evidence were refused. The unchanged legacy
rules also passed an eight-tick real static-network regression and
the existing kill/partition fault cases.

Reproduce:

```sh
cd research/distributed-simulation
python3 -m unittest discover -s . -p 'test_*.py' -v
unshare --user --map-root-user --net python3 transition_lab.py --output /tmp/new-sc61.json
python3 verify_transition_evidence.py /tmp/new-sc61.json
```

## Remaining gates

The experiment remains one physical Linux machine and one trusted
local authority. Worker and client processes are isolated locally;
there is no independent durable quorum, production consent or identity,
Byzantine/rollback protection, power-loss certification, native
Minecraft client or mod compatibility. The actual writer process
kills and explicit client disconnect/reconnect are not a full live
game-server crash or automatic network reconnect implementation.
No portal, real Minecraft block/inventory or arbitrary global game
mechanic is claimed. No production code, user worlds or release
configuration were modified. #62 next: resource withdrawal, scale-in,
worker loss and partition recovery from accepted checkpoints; retain
the production regional-hosting NO-GO.
