# Headless third voting replica

For a private world with two players who may not be online together, a
third authorized storage replica can supply the second vote of a three-member
majority. It does not need a Minecraft client, Java, Fabric, EULA acceptance,
or a graphics session. It is an ordinary signed world member, not a new
consensus role, directory service, or authority override.

## Prepare a dedicated device

Build the same reviewed version of the Rust CLI as the other peers, on the
target device, or use a verified binary for that OS and architecture:

```sh
cargo build --release --locked -p swarm-cli --bin swarmcraft
```

The CLI can run on a Linux server without the desktop application's webview.
Actual Raspberry Pi/NAS architecture and resource certification is separate
from local process tests. Allocate persistent storage for all accepted world
checkpoints and blobs. There is no general minimum disk capacity for an
arbitrarily large world.

Use a private, dedicated data root and device identity. Do not copy another
member's keys or use the same root on multiple machines:

```sh
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" init
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" identity
```

Create a signed invite from the current authority while its normal quorum is
available. Use its usual supported discovery/transport hints. Transfer the
invite privately; it is a bearer grant with an expiry. On the third device:

```sh
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" world join "$INVITE"
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" daemon --replica-only
```

Do not request `--host-candidate` for a dedicated storage-only member. Do
not configure a Java runtime or opt in to hosting. A non-banned admitted
member can vote without being eligible to become authority. Existing host
eligibility is not local consent.

Joining is not complete just because the join request was staged. The
current authority and required membership majorities must commit the
membership and the replica must verify the current checkpoint. Inspect:

```sh
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" world list
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" world status "$WORLD"
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" peers "$WORLD"
swarmcraft --data-dir "$HOME/.local/share/swarmcraft-replica" world hosting status "$WORLD"
```

Compare the accepted epoch, snapshot number/hash and membership against the
current authority. Do not count the device toward shutdown safety until
the current signed checkpoint is complete and its votes are reachable.

## Restart as a service

An example systemd unit is in
[`deploy/systemd/swarmcraft-replica.service`](../deploy/systemd/swarmcraft-replica.service).
Review the unit for your environment before installing it. It assumes a
dedicated unprivileged `swarmcraft` account, a verified CLI installed at
`/usr/local/bin/swarmcraft`, and a persistent private state directory
`/var/lib/swarmcraft-replica`. Initialize and stage the invite using that
account and **the same** state directory. Do not place the invite or private
keys in the unit, environment variables, or public logs.

The example binds the replication transport to UDP port 41000; adapt this
to the actual network. This is **not** a Minecraft port. For networks that
require explicit peer transport hints, use the existing `--peer` option
or supported signed invite bootstrap path. An authenticated peer connection
is still required; the service does not bypass firewall, NAT, discovery,
membership, or quorum constraints.

Use normal service restart/stop operations. The profile lock is tied to the
process lifetime; the systemd command reestablishes it on each restart. The
profile refuses to start when another Minecraft admission is active or a
previous runtime process has ambiguous ownership. Investigate that process
instead of deleting its ownership record. A normal daemon invocation without
`--replica-only` is not the headless profile. Run supported binaries of the
same version, rather than an old binary that predates profile enforcement.

The explicit profile:

- does not launch the migration supervisor or a Java process;
- keeps the identity, stored snapshots, signed membership, and existing
  durable consent revisions;
- excludes other supported hosting admissions on that data root;
- denies positive consent changes, local hosting permits, candidacy and
  transfer activation while the profile is active;
- leaves the ordinary authenticated replication and voting rules intact.

The profile is a cooperative local application guarantee, not an OS
sandbox against a privileged operator or modified executable.

## Availability and evidence

With three current non-banned members, a surviving player plus the online
replica can supply a majority **only** when the necessary exact accepted
state, signatures, and freshness requirements are satisfied. The player
must independently consent to hosting and meet runtime/capacity requirements.
The replica cannot play the world on its own. A stale or unreachable replica
does not turn an unavailable two-peer partition into a safe one.

The existing three-daemon recovery/wake test exercises an actual headless
process contributing a signed recovery vote, without a local runtime, and
resuming after restart. The CLI profile regression covers stale permit
removal, explicit opt-in denial, no runtime directory and restart. These
are local process tests; actual low-memory third-device, independently
networked gameplay and disk/resource measurements remain field acceptance.
