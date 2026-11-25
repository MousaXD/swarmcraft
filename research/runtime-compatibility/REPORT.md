# Runtime compatibility and qualification matrix

Date: 2026-11-25. Scope: Sabathon item #42.

The runtime adapter declaration and the new-world release policy are distinct.
The bridge currently declares Minecraft `~26.1.2`, Fabric Loader
`>=0.19.3`, and Java `>=25`. Those predicates are not evidence that
every matching tuple works.

| Minecraft | Fabric Loader | Reference Java | New Desktop world policy | Field qualification |
| --- | --- | --- | --- | --- |
| 26.1.2 | 0.19.3 | 25 | Pinned baseline candidate | Pending |
| 26.1.5 | 0.19.3 | 25 | Disabled; adapter-compatible, unqualified | Pending |
| 26.1.2 | 0.20.1 | 25 | Disabled; adapter-compatible, unqualified | Pending |
| 26.1.2 | 0.19.2 | 25 | Disabled; incompatible | Not offered |
| 26.2 | 0.19.3 | 25 | Disabled; incompatible | Not offered |

The non-baseline rows illustrate classification, not a claim of provider
availability. The actual catalogs remain authoritative for release existence.
The read-only `swarmcraft-runtime compatibility-matrix` output is saved in
[evidence/matrix-20261125.json](evidence/matrix-20261125.json).

## Local verification

- Shared protocol tests check the pinned tuple, near misses, version aliases,
  prereleases, lower Java versions, and build/adapter metadata consistency.
- Real import and native canonical Create gates refuse unsupported or unqualified
  tuples before publishing a world. Both direct and reviewed import were tested.
- Shipping and Svelte browser tests verify disabled options, precise labels,
  the pinned default, and the pending field-acceptance notice at all three
  supported test viewports.
- Existing signed world identities and technical runtime admission rules have
  not been rewritten. The legacy low-level CLI Create remains available for
  data-only protocol and tooling workflows; it does not qualify a game runtime.

## Field acceptance (not completed)

To qualify the pinned baseline, retain the exact source revision, package and
Fabric JAR hashes, platform, Minecraft version, Fabric Loader version, and
Java version. Build the real candidate with the Java 25/Gradle toolchain.
Use the existing `scripts/acceptance/clean-machine-live.sh` gate on an
isolated authorized test environment, with explicit Minecraft server EULA
acceptance and hosting consent. Verify actual Fabric launch, nonce-bound
authenticated readiness, canonical save and shutdown, snapshot verification,
restart/restore and the stable verified gateway. Its Minecraft status
exchange is not an authenticated player login.

Separately run the real client journey using a licensed compatible Java
Edition client and the operator's own launcher/account. Require authenticated
login through the fresh verified gateway, gameplay, save/restore and a new
connection after restart. Capture redacted evidence; do not retain credentials,
tokens, or private saves in the repository. Repeat for every newly proposed
exact tuple before widening the selection policy. Neither a mocked runtime,
a vanilla capacity benchmark, nor provider metadata alone qualifies a tuple.

The current checkout did not have the built candidate Fabric JAR or a
configured Java 25/Gradle build environment. The earlier item #35 real vanilla
server results are separate evidence and are not credited as a Fabric or
authenticated-client acceptance pass. No new real game launch was performed
for this checkpoint.
