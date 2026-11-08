# Three real machines — SwarmCraft field certification

**Agent 05 / issue #85. Current verdict: BLOCKED.** This is a reproducible,
fail-closed field-test harness, not a claim that a real three-player run passed.
Do not close #85 or add this job to mandatory release gates without the complete
evidence and independent review.

## Scope and prerequisites

SwarmCraft's existing single-runner player journey and synthetic three-daemon
recovery tests are useful regressions, but do not certify three genuine Minecraft
clients on independent networks. This harness checks three **independently
installed Debian packages**, distinct remote machine identities, distinct
observed WAN egress, live signed backend state, verified canonical snapshots,
accepted recovery epoch/fencing, and a guarded authority process crash.

Prerequisites: #82/#83 packaged startup acceptance, #84 actual enforced
hosting consent (Bob opt-in; Carol 0 GB), and #87 genuine Tauri/Svelte desktop.
Provision three different Linux machines with real graphical desktops, real
Minecraft clients, legitimate logins, compatible Fabric/Java installations,
and three **separate external networks**. Cloud VMs on three WAN IPs do not
by themselves prove representative home NAT/CGNAT/mobile connectivity.

Use the exact same .deb artifact produced from the candidate integrated SHA;
retain its trustworthy build CI URL and SHA-256. Install that package using
the actual OS package manager on each machine. Verify the installed Tauri
binary and CLI sidecar locations and adjust the private config accordingly.
Use a new SwarmCraft data profile and independently initialize through each
installed GUI, never by cloning another machine's data.

Verify SSH host keys out of band. SSH must use pinned known_hosts and key-only
BatchMode. Keep a private copy of config.example.json **outside the repo**.
Fill exact 40-hex commit, 64-hex package SHA, random 32-byte hex fingerprint
salt, SSH destinations and absolute package/binary/CLI/data paths. Do not
publish the config or salt. The probe pseudonymizes peer IDs, machine IDs
and public egress addresses; it never exposes invitation strings or raw CLI
logs.

## Optional exact-package provisioning and bounded cleanup

A dedicated test operator can install the same source-provenance-linked .deb
on all three hosts with the optional script below. Before doing so, provision
the dedicated /opt/swarmcraft-e2e/ directory on each host as writable by the
authorized SSH operator and allow noninteractive sudo for the exact dpkg
install action. It does **not** configure identities, modify signed worlds,
download Minecraft, or approve the EULA.

```bash
python3 scripts/e2e/three_machine/ops.py provision --config "$CONFIG" \
  --deb /private/build/candidate.deb --output "$OUT" \
  --confirm INSTALL-THREE-REAL-TEST-MACHINES
```

The command fails if the local or remote SHA differs from the expected
candidate package. If one host fails, provision.json records which earlier
hosts were installed; it deliberately does not attempt unsafe uninstall
rollbacks. The field operator starts genuine native GUI and Minecraft clients
in each machine's logged-in desktop session.

After the test and evidence review, staged .deb files can be deleted with
a second **explicit** confirmation. The installed application, players,
canonical worlds and game data are never removed by this script:

```bash
python3 scripts/e2e/three_machine/ops.py cleanup-stage \
  --config "$CONFIG" --output "$OUT" \
  --confirm DELETE-ONLY-STAGED-E2E-DEBS
```

Remote capture additionally records SHA-256 digests and sizes for a bounded
set of log files under each test data profile's logs directory. Full sensitive
log contents remain local for secure operator inspection; they are not
published in the sanitized JSON.

## Running the scenario

Run from repository root. Set CONFIG to your private config path, OUT to a
private directory, WORLD_ID to the actual signed 64-hex world ID, and PIDS
to a private JSON file of current, genuinely observed process IDs:

    {"alice":{"desktop":1234,"client":1235,"server":1236},
     "bob":{"desktop":2234,"client":2235},
     "carol":{"desktop":3234,"client":3235}}

Launch each installed SwarmCraft desktop with SWARMCRAFT_DATA_DIR set to the
configured isolated profile (without giving the runner your private key or
secrets). The probe requires the native desktop process's actual executable
and data-root environment to match the installed package; the Java server
must have the expected world-bound runtime working directory and a genuine
Java executable with a -jar command. This prevents unrelated processes from
standing in as player/host proof.

Update process IDs after a restart or takeover; false, missing or wrong PIDs
must fail. Process presence is auxiliary evidence, not proof of a successful
Minecraft login, join or gameplay. The seven phase captures must happen
**in their real chronological order**; the harness does not fabricate them.

```bash
python3 scripts/e2e/three_machine/run.py preflight --config "$CONFIG" --output "$OUT"

# Alice opens the installed GUI, initializes identity, explicitly accepts the
# Minecraft server EULA, creates a signed world, starts genuine Fabric/Minecraft.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage alice_created --world "$WORLD_ID" --pids "$PIDS"

# Bob installs/initializes separately, accepts Alice's signed invitation,
# joins with a real Minecraft client and chooses an allowed hosting budget.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage bob_joined --world "$WORLD_ID" --pids "$PIDS"

# Carol independently accepts invitation, joins real client, sets 0 GB host.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage carol_joined --world "$WORLD_ID" --pids "$PIDS"

# First observe a real established direct path (or legitimate successful
# DCUtR upgrade) between independently networked peers; it must be backend
# connectivity state, NOT just a configured bootstrap address or local ping.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage transport_direct --world "$WORLD_ID" --pids "$PIDS"

# Then deliberately make direct communication unavailable while retaining a
# real configured and reachable relay. Observe active relay connectivity.
# Do not rely on test-loopback or a configured-but-unused relay.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage transport_relay --world "$WORLD_ID" --pids "$PIDS"

# Field operator applies/reverts measured packet loss and a genuine peer
# disconnection at the network/device boundary, captures the transport
# outcome and verifies all clients reconnect without split-brain.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage transport_disrupted --world "$WORLD_ID" --pids "$PIDS"

# Interact in game, observe deterministic marker in all 3 real clients, wait
# until all 3 store the same signed/verified latest accepted checkpoint.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage checkpoint --world "$WORLD_ID" --pids "$PIDS"

# On a DISPOSABLE dedicated test profile, record Alice's live Tauri GUI,
# replication daemon and genuine Minecraft server JVM PIDs. An isolated
# application outage requires all three components to die; Java-only crash
# is explicitly rejected and cannot masquerade as a genuine authority loss.
python3 scripts/e2e/three_machine/run.py crash --config "$CONFIG" --output "$OUT" \
  --world "$WORLD_ID" --desktop-pid "$ALICE_DESKTOP_PID" \
  --daemon-pid "$ALICE_DAEMON_PID" --server-pid "$ALICE_SERVER_PID"
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage alice_crashed --world "$WORLD_ID" --pids "$PIDS"

# Confirm surviving-majority quorum, Bob promoted with higher epoch/fencing,
# real Fabric running on Bob, and genuine Minecraft client reconnection.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage bob_recovered --world "$WORLD_ID" --pids "$PIDS"

# Bring Alice back; prove she is nonauthoritative, fully synchronized, fenced.
python3 scripts/e2e/three_machine/run.py capture --config "$CONFIG" --output "$OUT" --stage alice_returned --world "$WORLD_ID" --pids "$PIDS"

# Only after independent human review of genuine native desktop, real game
# client, selected network transport and budget evidence:
python3 scripts/e2e/three_machine/run.py verify --config "$CONFIG" --output "$OUT" --review "$REVIEW_JSON"
```

The crash injector requires three distinct PIDs: the test-profile native
Tauri desktop, its SwarmCraft replication daemon, and the real Minecraft
server JVM in the exact world runtime directory. It checks executable identity
and/or test-profile environment, validates all three before stopping them,
freezes their processes to prevent partial restart, and sends SIGKILL to each.
A Java-only or desktop-only crash is **not** accepted as authority loss.
This is **a full isolated application-process outage, not physical machine
power loss**; literal machine/ISP power-failure field evidence is a separate
required exercise. It never uses wildcard pkill, unrestricted rm, mocked Java,
pre-seeded replicas or localhost-only peers.

## Host consent and real JVM heap observations (#84 dependency)

The harness expects the eventually integrated Agent 03 local consent contract
at `control/<world-hex>/hosting-consent.json`, containing schema version 1,
the actual fixed-width world ID and local peer identity, explicit enabled
state, `heap_mib` and positive consent revision. It **never creates or edits**
these records. Missing, malformed, wrong-device or wrong-world records are
reported unavailable, and the final verifier stays BLOCKED.

At the signed three-peer checkpoint, Alice and Bob must each have explicit
opt-in with an allowed nonzero heap, while Carol must have a visible explicit
revocation/0-MiB player-only record. The verifier checks that Alice's running
Minecraft server process has a real `-Xmx` flag equal to Alice's allowed
heap and Bob's recovered Java process uses Bob's independently chosen limit.
Carol may never become a Java authority while remaining player-only.
Consent snapshots and revision are rechecked after Bob's takeover; this
does not replace authenticated quorum or signed recovery.

The `-Xmx` JVM argument is a **heap** limit only, not a guaranteed total
RSS/cgroup limit. Real runtime memory pressure and OS-level process cap
behavior still require explicit independent evidence and review. Agent 03's
own contract marks actual policy integration, GUI support and OS-level hard
memory enforcement as unfinished; this harness does not bypass those safety
gaps or make them green by inspecting a JSON file.

## Evidence and final review

The "transport_direct" and "transport_relay" captures now independently
require actual backend connectivity to report direct/hole-punched and active
relay transport respectively. Configured but inactive relays are explicitly
rejected. If no genuine direct path is available on this network set, do
**not** fake it: run a separate independent-network scenario where possible,
or keep the overall certification BLOCKED. Packet-loss/disconnect evidence is
captured as a stage but still requires external measured-loss/router logs
and real client reconnection review; a JSON stage alone does not prove it.


Record native GUI installation/first-run/EULA/invitation screens, genuine
Minecraft server and client versions and logs, signed world ID, real player
session and deterministic shared marker screenshots/video from all three
clients, accepted snapshot hash/state root, epoch and fencing changes, Bob's
consent/budget enforcement, Carol's 0 GB role, real direct or DCUtR/relay
transport selection, client reconnections, stale-Alice rejection and absence of
duplicate authority. Include a separately reviewed test for relay fallback,
packet loss/disconnections and operator-confirmed network classes. Redact IPs,
tokens, invitations, keys and account names before publishing artifacts.

### Mandatory redacted media manifest (role-specific)

The reviewed evidence must now include **23 distinct image/video files** with
SHA-256 checksums in the private `$OUT/evidence/` directory. The
`review.example.json` file includes an `artifacts` array; populate it with
exactly one object per required `kind` and `role`:

```json
{
  "kind": "marker_before",
  "role": "alice",
  "file": "evidence/marker_before/alice.png",
  "sha256": "64 lowercase hex characters of the actual file"
}
```

Required evidence categories:

- `native_install` for Alice, Bob, Carol.
- `client_join` for Alice, Bob, Carol.
- `marker_before` and `marker_after` for Alice, Bob, Carol; show the same
  deterministic world state in all **genuine** Minecraft client sessions.
- `signed_invite` for Alice, Bob, Carol (show that joins use the genuine signed
  invitation flow, but **redact invite tokens**).
- `hosting_budget` for Bob and Carol (real backend-enforced consent and
  explicit 0 GB player role, not a decorative UI selection).
- `eula_consent` for Alice.
- `transport_direct` and `transport_relay` for Bob, captured from the
  actual selected libp2p connection (not simply a configured relay).
- `disconnect_reconnect` for Bob after network disruption.
- `alice_crash` and `alice_fenced` for Alice.

Images must be PNG/JPEG; videos may be WebM/MP4. Every file must have a
matching lowercase SHA-256, a correct media signature and a distinct digest.
Symlinks, paths escaping `$OUT/evidence`, missing items and reused images are
rejected; the final verdict includes a digest of the checked manifest.
Ensure the reviewer checks the **actual content**; matching file hashes
cannot prove genuine gameplay, privacy redaction, or authenticity on their own.
Store originals with appropriate privacy controls; only publish explicitly
redacted proof. Do not confuse the synthetic PNGs used by unit tests with
real acceptance artifacts.

An independent reviewer fills review.example.json with booleans **only after**
examining the corresponding evidence and adds a public sanitized evidence URL.
The automated verifier additionally checks exact package/commit, three distinct
machines/egress routes, three signed memberships, valid replicated snapshot,
Bob as proven successor before Alice is killed, legitimate recovered generation,
restored accepted checkpoint and Alice's nonauthoritative return. Missing
evidence blocks certification. Human checkbox attestations are not independent
machine cryptographic attestations; a READY verdict must be accompanied by the
actual reviewed artifacts.

## Isolated optional GitHub Action

The new .github/workflows/three-machine-certification.yml runs **negative-only
harness unit tests** on relevant PR changes. When manually dispatched it
performs **preflight only**, using three repository secrets:

- SWARMCRAFT_E2E_CONFIG_JSON — private config matching workflow source commit.
- SWARMCRAFT_E2E_SSH_PRIVATE_KEY — scoped field-lab SSH identity.
- SWARMCRAFT_E2E_SSH_KNOWN_HOSTS — independently verified host-key pins.

A green preflight means only that three installed .deb packages and network
endpoints passed preliminary checks. It does **not** mean the complete E2E
scenario passed. The currently interactive game/GUI test must be performed on
real machines and its external evidence reviewed before the verifier can
return READY. No full E2E CI run URL exists until the real lab is provisioned.

## Local negative unit checks

```bash
python3 -m compileall -q scripts/e2e/three_machine
python3 -m unittest discover -s scripts/e2e/three_machine -p 'test_*.py' -v
```

Clean up **only** the named test-owned processes and explicitly isolated
profiles/VMs after evidence retention. Stop worlds via their safe durability
barrier if possible; never silently delete user saves.

**Current blocker:** no connected three-machine field lab, E2E credentials or
reviewed real-client artifacts were supplied, and the #84/#87 integrated
candidate remains to be certified. Thus no field PASS, no certified package SHA,
and no full E2E CI URL can truthfully be reported.
