# #56 reopened feasibility: chosen experimental runtime

2026-11-25. **Research track only.** This design is selected before the
networked experiment. It does not alter the 2026-11-08 Fabric NO-GO or
authorize production region execution.

## Choice

Use a purpose-built deterministic region runtime (Python 3 standard
library for the reference lab), with one fixed region partition of a
single synthetic world, independent worker processes, actual authenticated
framed TCP, and a deliberately synchronous trusted coordinator. This
allows real inter-worker network experiments and differential correctness
tests without running an unqualified Minecraft fork, rewriting saves,
or giving a new process canonical authority.

Candidate evaluation, checked against upstream documentation on
2026-11-25:

- [Folia architecture](https://docs.papermc.io/folia/reference/overview/):
  region-local ownership and a global scheduler are useful references,
  but Folia parallelizes within one process. It does not provide the
  independent remote writer and crash protocol required here.
- [MultiPaper README](https://github.com/MultiPaper/MultiPaper/blob/main/README.md)
  and [upstream warning](https://multipaper.io/): the described beta is
  a 1.20.1 Purpur fork with a master coordinating chunks and storing
  data. It is not our 26.1.2 Fabric runtime; upstream explicitly warns
  of item duplication and chunk corruption. Do not adopt this fork as
  a compatible or certified SwarmCraft runtime.
- [Minestom suitability](https://minestom.net/docs/when-to-use):
  a custom server/protocol foundation is possible, but the framework
  lacks most vanilla survival mechanics and is not Fabric compatible.
  No reviewed remote-region transaction or compatibility proof exists.
- Keep the single-authority Minecraft/Fabric product path unchanged.
  The reference lab tests a constrained custom state machine rather
  than pretending that another server's version/claims establish
  compatibility. A future Minecraft-facing implementation needs a new
  runtime selection after proof and compatibility review.

## Experimental rules and test plan

A ring of eight fixed regions contains bounded cell signals, integer
resource banks, and agents with unique IDs, positions and inventories.
Each tick reads the previous neighbor edges, updates cell signals,
processes deterministic pickup/drop and one-cell movement, and transfers
a unit of resource to the next cell when its rule permits. Agents move
back and forth deterministically; boundary-heavy initialization places
them at crossing points. The world tick is one global logical clock.
No claim is made about Minecraft redstone, hoppers, players, physics,
portals, chunk generation, mods, authentication, saves or wire protocol.

Workers hold their own state and execute their owned regions. Three
bounded barrier phases prepare outgoing movements, seal destination
updates, then acknowledge the candidate cut. The coordinator only
routes events, checks region scope/counts/resource conservation, combines
ordered region digests and accepts the cut after all acknowledgments.
A missing or malformed response poisons the entire run; no retry,
reassignment or advancement after a fault. Compare *every* accepted
world root to an independently traversed monolithic serial reference.
No automatic production fallback or authority publication.

Use fresh mapped-user/network namespaces on this workstation: distinct
worker namespaces connected to the coordinator by veth links; no host
network or actual user world. Record local CPU/RSS, application frame
and actual interface bytes, measured echo RTT, tick p50/p95/p99, 50 ms
deadline misses and accepted tick count. Compare one, two and four
workers on identical seeds, spread/cluster/boundary load, and kernel
netem delay. Test killed and partitioned workers during a boundary
transaction. Retain actual results including negative outcomes.

This is a trusted single coordinator and honest-process experiment.
HMAC and per-direction sequence bind private lab connections, not
SwarmCraft identity, quorum, Byzantine execution or encryption. A
coordinator or worker failure can lose the entire experiment; local
acknowledgments are not a durable consistent cut or safe recovery.
Regions freeze on failure. Separate network namespaces are not
independent physical hosts, disks, clocks or failure domains.

Production GO still requires the prior RFC's safety and gameplay
gates, real independent machines, authenticated clients and supported
Minecraft mechanics/mods, >=25% sustained spread-load gain at 20 TPS
with p95 below 50 ms within admitted resource budgets, zero detected
corruption and safe recovery. A lab pass cannot satisfy those gates.
