# Managed Minecraft listener and account admission policy

## Supported baseline

The shared authority runtime creates a new managed `server.properties`
before Java starts:

- `server-ip=127.0.0.1`; a durable world-specific port in 40000–59999,
  separate from the player's local gateway address.
- `online-mode=true` and `enforce-secure-profile=true` preserve normal
  Mojang authentication and account UUIDs; no username-derived offline
  identities or authentication downgrade.
- RCON and query disabled. Existing runtime files are not silently
  overwritten by the property writer, and a busy assigned port is refused.
- The game gateway, not public exposure of the Minecraft listener, is the
  supported remote transport. Signed current membership, fresh quorum,
  authority fence, consent, and authenticated Fabric readiness remain
  necessary for admission and ongoing streams.

On Linux the supervisor additionally examines the actual child-owned TCP
listener in the kernel after authenticated startup and after every live
Fabric readiness challenge. It accepts the exact IPv4 loopback address or
its IPv4-mapped IPv6 representation, and refuses wildcard, foreign-owner,
missing, or ambiguous listeners on the reserved port. This is a point-in-time
check, not a sandbox against malicious local executables, mods, or another
process running as the same OS account. Windows/macOS retain configured
loopback and authenticated readiness checks; an equivalent native socket
ownership gate has not yet been implemented or certified there.

## Identity boundary

A SwarmCraft peer is a device signing key. A Minecraft player is an
independently authenticated Mojang/Microsoft account UUID. Neither a
nickname, a contact label, a device public key, nor a user-supplied UUID
proves control of the other identity.

V1 currently enforces device membership at the remote game gateway. It
intentionally writes `white-list=false` and `enforce-whitelist=false`;
there is no verified canonical Minecraft-account roster. Enabling a
whitelist from device IDs would deny legitimate players or create false
authorization. A process that can connect directly to the host's private
loopback listener is not yet subject to a Fabric per-session device binding.
Do not describe the baseline as the complete Minecraft login allowlist,
or publish the listener directly to LAN/Internet.

## P1 admission design (not activated)

1. Version the canonical world account-binding policy. Binding changes
   need explicit account-holder action, authority approval and reviewed
   durable quorum publication, preserving tombstones and role boundaries.
   No unsigned launcher hints or local preferences can grant access.
2. Establish initial account possession only through the actual
   online-mode authenticated Minecraft profile over a device-authenticated
   gateway session, using a nonce-bound, time-limited enrollment flow.
   The authority must explicitly confirm the intended peer/UUID pair.
   Enrollment must not grant ordinary gameplay before committed approval.
   Account replacement is a new reviewed transition, not a silent rewrite.
3. Correlate each admitted gateway stream to the exact live local server
   socket and authenticated Fabric IPC runtime instance before forwarding
   login bytes. A short-lived single-use registration must bind the
   world, authenticated application/transport identity, current membership
   and account-binding hashes, authority generation, fresh challenge and
   exact connection tuple. A direct loopback login without that registration
   is denied. Do not rely on an address alone, untrusted client metadata,
   or a reusable registration.
4. At the native authenticated login hook, compare the Mojang-verified
   profile UUID with the active canonical binding for that exact device
   session. Consume the registration; recheck current authorization,
   epoch, fence, expiration and permit. Close sessions on relevant
   membership, account mapping, route or consent invalidation.
5. Preserve account UUIDs and world data across authority transfer without
   transferring Microsoft credentials, access tokens, launcher accounts,
   or IPC secrets. A new host must load verified canonical policy and
   independently establish fresh runtime/session evidence.
6. Negotiate the mandatory feature at both gateway and Fabric boundaries
   before activating it. Mixed-version peers incapable of enforcing
   login policy cannot silently accept the world in a weaker mode.

Activation requires local real-Fabric compilation and native authenticated
login acceptance: matching account, mismatched account, unbound device,
direct loopback bypass, replayed registration, expired challenge, retired
device, changed account binding, transfer, and restart. Preserve the user's
launcher sign-in; do not automate acquiring or exposing credentials.
The current Java target is 25. A build or socket mock alone does not prove
Minecraft's login hooks enforce this policy.

## Local validation boundary

The Linux kernel socket tests use actual sockets owned by the test process
and independent child processes. They establish address/ownership checks,
not an installed Minecraft login. A separate host-process fixture exercises
wildcard-listener denial before runtime readiness or checkpoint publication.
On the local workstation this fixture and the broader hosting scenarios
could not execute their hosting paths because available RAM was below the
unchanged admission requirement (4543 MiB for the selected 512 MiB heap).
Run them again with sufficient available memory; do not weaken admission to
make a test pass. No hosted CI was dispatched for this milestone.
