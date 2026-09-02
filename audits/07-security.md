# Auditor 7 — Security / Trust Boundary Audit

Repository: `MousaXD/swarmcraft`  
Audit branch: `audit/security`  
Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`  
Audit type: independent cross-cutting security review  
Production changes: **none**

## Verdict

**VERDICT: FAIL**

The audited tree contains a confirmed authorization boundary failure: after ordinary signed peer authentication, a non-member can request private-world operational metadata, including the full `WorldDescriptorV1` member list, when it knows the world ID. This violates the repository's stated private-world and membership boundaries and is rated **HIGH**.

The audit also found a CurseForge credential-handling weakness, a defense-in-depth gap in the privileged Tauri runtime-launch surface, and predictable temporary-file writes that follow symlinks. No arbitrary remote code execution, private-key exfiltration, signature bypass, or canonical-history forgery was confirmed in this audit.

Exact-head CI was green at the audited commit, including process-level hostile-network tests, fuzz smoke, network soak/player acceptance, and a root-workspace RustSec job. Those gates do not exercise the confirmed non-member metadata query path and therefore do not change the verdict.

---

## Baseline gate

The audit prompt required stopping if live `main` had moved from:

`354be3b1066428ecab6987590b7c7dbd80fe0870`

Live GitHub `main` was verified at exactly that SHA before the audit branch was created. `audit/security` was then created directly from that commit.

---

## Threat model

This review assumes the following actors can be hostile:

1. **Unaffiliated internet/LAN peer** able to create its own SwarmCraft identity and complete the signed `PeerHello` handshake.
2. **Current member** that is malicious while still possessing legitimate membership.
3. **Former, removed, or banned member** that retains a world ID and historical network coordinates.
4. **Provider/upstream service** that returns malicious metadata, redirects, or artifact bytes.
5. **Compromised desktop frontend/webview** that can call registered Tauri commands.
6. **Local process or local OS user** racing predictable files in shared temporary storage.
7. **Malicious imported world/mod content** attempting traversal, symlink abuse, parser exhaustion, or inconsistent canonical state.

Security goals used for judgment are also consistent with `SECURITY.md`: peer authentication must not be confused with authorization; private-world state must not be disclosed to arbitrary peers; local control channels should be authenticated; filesystem restore/import paths must reject traversal and symlink escapes; untrusted message sizes should be bounded; and secret material should not leak through logs or process interfaces.

---

## Attack surface map

| Surface | Entry points | Trust boundary | Important controls observed | Residual concern |
|---|---|---|---|---|
| libp2p replication daemon | QUIC/TCP, mDNS, relay, request-response | arbitrary network peer -> authenticated application peer -> world member | signed `PeerHello`, peer/public-key binding, request size validation, signed canonical records | several metadata requests authorize only at authentication layer, not membership layer |
| discovery service | public/unlisted lookup, friend presence | arbitrary authenticated discovery peer | signed announcements, private worlds not published, public search filters public only | separate replication metadata leak still bypasses discovery privacy intent |
| local Fabric bridge | loopback TCP | Minecraft/Fabric child process -> authority runtime | random 256-bit token, loopback peer check, bounded line size, timeouts | token is passed in child environment; acceptable for same-user child-process trust model but should remain non-loggable |
| storage / restore / import | world directories, manifests, blobs, imported saves | untrusted local/imported filesystem content -> canonical storage | symlink rejection, portable relative paths, hash verification, staged publication, restore parent validation | some unrelated temp-report paths do not use equivalent no-follow discipline |
| package/provider acquisition | Modrinth, CurseForge, Mojang, Fabric, Adoptium, release assets | remote provider -> local runtime | HTTPS restriction, canonical hashes, max artifact size, safe JAR basename, runtime compatibility checks | CurseForge secret is placed in curl argv/custom header; redirect policy can forward it |
| Desktop/Tauri IPC | registered `invoke` commands | webview JS -> native filesystem/process authority | local bundled frontend, CSP, no shell-string concatenation, most dynamic DOM values use `textContent` | frontend-supplied advanced runtime executable path can reach `Command::new` through privileged command surface |
| subprocesses | `curl`, Java, sidecars, platform folder opener, `tar` | native backend -> external executable | arguments passed as argv, not shell text; sidecar names fixed | external tool behavior/version matters; sensitive curl header handling is unsafe |
| identity keys | application Ed25519 and libp2p transport keys | filesystem -> long-lived identity | `OsRng`, Unix `0600`, write/sync/rename, secret omitted from `Debug` | explicit ACL hardening is Unix-specific; non-Unix protection relies on parent/profile ACLs |
| CI/dependencies | Cargo/desktop/Fabric workflows | dependency supply chain -> release | root `Cargo.lock`, RustSec, fuzz smoke, multiplatform builds | root RustSec scope does not directly audit the separately locked/excluded Desktop crate |

---

# Findings

## SEC-07-001 — HIGH — Authenticated non-members can read private-world replication metadata

**Severity:** HIGH  
**Confidence:** High  
**Affected components:** `crates/swarm-cli/src/daemon.rs`, `crates/swarm-network/src/node.rs`, `crates/swarm-network/src/wire.rs`, `crates/swarm-protocol/src/lib.rs`

### Boundary / invariant

Completing the signed peer handshake establishes **identity**, not **world membership**. Private-world operational metadata must be disclosed only to authorized current members, except for the narrow signed invite/join information intentionally carried by an invite.

### Evidence

`SwarmNode::next_event` accepts a correctly signed `PeerHello` and stores a mapping from transport peer to application peer. Once that mapping exists, all non-Hello requests are surfaced as authenticated inbound requests.

In `daemon.rs::handle_request`, these request arms do not call `authorize_member` and do not otherwise verify that `application_peer` is a current, unbanned member of the requested world:

- `WireRequest::WorldStatus { world_id }`
- `WireRequest::HostCapability { world_id }`
- `WireRequest::WorldDescriptor { world_id }`

`WorldDescriptor` directly does:

```rust
let descriptor = storage.load_world_descriptor(world_id).ok();
node.respond(channel, WireResponse::WorldDescriptor(descriptor))?;
```

By contrast, blob-transfer operations such as `MissingBlobs` and `BlobChunk` explicitly call `authorize_member(storage, world_id, application_peer)?`, proving the codebase already has the required authorization primitive but does not apply it to these metadata endpoints.

`WorldDescriptorV1` contains:

- `world_id`
- compatibility fingerprint
- the complete `members: Vec<WorldMemberV1>`
- every member peer ID
- every member public key
- authority-eligibility flags
- banned flags
- preferred replication factor

`WorldStatus` and `HostCapability` also reveal current replication/hosting state useful for topology and availability reconnaissance.

### Attack / failure scenario

1. Attacker creates a normal SwarmCraft identity.
2. Attacker connects to a known daemon address and completes the valid signed `PeerHello` handshake.
3. Attacker knows a private world ID. Realistic sources include a leaked/old invite, previous legitimate membership, local history, logs, or an out-of-band disclosure.
4. Attacker sends `WorldDescriptor { world_id }`, `WorldStatus { world_id }`, or `HostCapability { world_id }`.
5. The daemon responds without checking current membership.

A removed or banned member is an especially strong reproduction case because it naturally retains the unguessable world ID but is no longer authorized.

### Impact

This is a private-world metadata disclosure and membership-boundary bypass. It exposes current member identities/public keys and host/replication information to peers that are authenticated globally but unauthorized for that world. It weakens privacy, enables membership/topology reconnaissance, and contradicts the project's separation between discovery and canonical membership.

No write/canonical-history modification was demonstrated through this path, so this is HIGH rather than CRITICAL.

### Existing coverage

The exact-head CI contains hostile-network and handshake-hardening tests, but the reviewed tests do not prove that a validly authenticated **non-member / removed / banned** peer is denied `WorldDescriptor`, `WorldStatus`, and `HostCapability`.

### Required remediation

Centralize world-scoped authorization and make the default fail closed. Before responding to these endpoints, require that `application_peer` is a current descriptor/membership member, is not banned, and has the expected public-key binding. If any request truly must be pre-membership, define a separate minimal protocol response that contains only the data required for the join flow and prove that it cannot expose private membership/topology.

### Test required to close

Add process/network tests covering at minimum:

- authenticated stranger + known private world ID -> denied/no private metadata;
- removed member -> denied;
- banned member -> denied;
- current valid member -> permitted;
- public discovery still works through the discovery protocol without granting replication metadata access.

---

## SEC-07-002 — MEDIUM — CurseForge API key is exposed in curl argv and can be forwarded as a custom header across redirects

**Severity:** MEDIUM  
**Confidence:** High  
**Affected component:** `crates/swarm-cli/src/provider_runtime.rs::curseforge_json`

### Boundary / invariant

`SWARMCRAFT_CURSEFORGE_API_KEY` is a machine-local credential. It should not be exposed in process listings and must not be forwarded to redirect destinations outside the intended CurseForge API origin.

### Evidence

`curseforge_json` launches external curl and places the secret directly into an argv element:

```rust
command.arg("-H").arg(format!("x-api-key: {api_key}"));
```

The same curl invocation enables redirects with `-L` and only restricts protocol to HTTPS:

```text
-L --proto =https
```

There is no `--proto-redir` host policy, no manual redirect loop that revalidates the origin, and no mechanism that keeps the key out of process arguments.

curl's own documentation warns that headers supplied through `-H/--header` are used on subsequent HTTP requests when redirects are followed and may be sent to other hosts. curl has special cross-origin protection for `Authorization:` and `Cookie:`; `x-api-key` is a generic custom header and does not receive that special handling. See curl's current `--header` documentation and known-risks page:

- `https://curl.se/docs/manpage.html`
- `https://curl.se/docs/knownrisks.html`

curl's tutorial also documents using stdin/config mechanisms to avoid sensitive options being visible in process tables.

### Attack / failure scenarios

**Redirect leak:** a compromised/misconfigured CurseForge API endpoint, DNS/TLS trust compromise, or other upstream condition returns an HTTPS redirect to another origin. `curl -L` follows it and the custom `x-api-key` header can accompany the redirected request.

**Local process-table leak:** on platforms/configurations where another local user can read process command lines, the key is briefly visible in curl argv.

### Impact

Exposure of the CurseForge credential can allow unauthorized use of the user's API key, quota consumption, or actions available to that credential. This does not directly expose SwarmCraft identity private keys or canonical history, so MEDIUM is appropriate.

### Positive controls

- the key is obtained from environment configuration rather than committed source;
- errors do not print the key;
- provider artifact bytes are verified against canonical SHA-1/SHA-256/SHA-512 after download;
- redirects are restricted to HTTPS protocol, which reduces but does not eliminate cross-origin leakage.

### Required remediation

Prefer an in-process HTTP client with an explicit redirect policy that permits only the expected CurseForge origin(s) for authenticated API requests and stores the credential in an HTTP header outside process argv. If curl must remain, disable automatic redirects for authenticated API calls and follow redirects manually only after revalidating exact scheme/host/port. Do not put the API key in command-line arguments.

### Test required to close

Run a local HTTPS redirect fixture and prove that:

1. the first request contains `x-api-key`;
2. a cross-origin redirect is rejected or followed without the secret;
3. the secret does not appear in child-process argv/logging.

---

## SEC-07-003 — MEDIUM — Privileged Tauri runtime configuration trusts frontend-supplied executable paths

**Severity:** MEDIUM  
**Confidence:** High for the primitive; Medium for exploitability without a separate frontend compromise  
**Affected components:** `apps/desktop/src-tauri/src/main.rs::configure_world_runtime`, registered Tauri invoke surface, `crates/swarm-cli/src/main.rs::WorldCommand::RuntimeConfigure`, `crates/swarm-cli/src/migration.rs::launch_server`

### Boundary / invariant

The Desktop webview should be treated as a less-privileged presentation layer. A frontend compromise should not automatically become arbitrary native process selection.

### Evidence

The registered Tauri command `configure_world_runtime` accepts frontend-controlled strings for:

- `java`
- `server_jar`
- `mod_jar`
- `game_endpoint`

It forwards those values as separate sidecar CLI arguments. The CLI `RuntimeConfigure` handler verifies world existence and explicit EULA acceptance, then persists the paths without restricting them to a managed runtime root or proving that `java` is the installer-selected Java executable.

Later, `migration.rs::launch_server` executes:

```rust
let mut command = Command::new(java);
command.arg("-jar").arg(server_jar).arg("nogui").current_dir(runtime);
command.spawn()?;
```

There is no shell-string injection here, which is good. The security issue is the stronger primitive: a compromised frontend can select the executable path itself.

### Exploitability assessment

No direct XSS was found in the reviewed primary desktop rendering paths: dynamic backend/provider values are generally inserted with `textContent`, while reviewed `innerHTML` assignments are static templates. The app uses a restrictive local CSP and does not intentionally load a remote frontend. Therefore this finding is **not** rated as direct RCE and is MEDIUM rather than CRITICAL/HIGH.

However, the native command surface removes an important containment layer. Any future DOM injection, compromised bundled frontend asset, or unintended webview navigation would inherit native process-selection authority.

### Required remediation

Split managed and advanced runtime configuration:

- normal Desktop flow should accept an opaque backend-selected runtime profile/world ID, not arbitrary executable paths;
- backend should resolve the managed Java/server/mod paths itself and verify they are beneath expected managed roots with expected hashes;
- if an advanced/manual path mode is intentionally supported, gate it behind an explicit high-friction local user action and keep it separate from commands reachable by ordinary app rendering;
- consider Tauri capabilities/window scoping so only the intended local app window can invoke the most privileged commands.

### Test required to close

Add native/Tauri contract tests proving that the standard frontend cannot configure `java` outside the managed runtime root and that privileged commands are unavailable to an untrusted/secondary window or navigation context.

---

## SEC-07-004 — LOW — Predictable temporary diagnostics path is written with symlink-following `fs::write`

**Severity:** LOW  
**Confidence:** High for behavior; Low-to-Medium for practical cross-user exploitability because OS temp-directory protections vary  
**Affected components:** `apps/desktop/src-tauri/src/runtime.rs`, `crates/swarm-network/src/diagnostics.rs::persist_json_snapshot`

### Evidence

Desktop chooses a diagnostics JSON path derived only from its process ID, effectively:

```text
<system temp>/swarmcraft-connectivity-<pid>.json
```

The daemon receives that path through `SWARMCRAFT_CONNECTIVITY_DIAGNOSTICS_JSON`. `ConnectivityDiagnosticsV1::persist_json_snapshot` then performs:

```rust
fs::write(path, bytes)
```

There is no `create_new`, no no-follow open, no ownership check, and no atomic exclusive temporary file. A pre-existing symlink is followed by normal filesystem semantics.

`provider_runtime::temporary_path` uses a stronger nonce component (PID plus current nanoseconds) but similarly relies on path naming rather than exclusive no-follow creation before handing a path to curl.

### Impact

On an OS/configuration where an attacker can precreate/race that temporary pathname and symlink protections do not block following it, SwarmCraft can truncate/replace another file writable by the SwarmCraft user. Common hardened Linux sticky-directory rules can reduce cross-user exploitation; user-specific temporary directories on other platforms can also reduce exposure. The code does not enforce the invariant itself, so it remains a local hardening flaw.

### Required remediation

Create the diagnostics file with an OS-safe exclusive temporary-file API in a user-private directory, keep the opened handle or use an atomic no-follow replacement strategy, and validate ownership/type before reuse. Prefer storing this report beneath the existing private SwarmCraft data root rather than a shared temp directory.

### Test required to close

Add a Unix symlink-precreation test and platform-specific tests for Windows/macOS temp behavior. Prove that a pre-existing symlink or non-regular file cannot redirect diagnostics writes.

---

## SEC-07-005 — LOW — RustSec gate does not directly cover the separately locked Desktop dependency graph

**Severity:** LOW  
**Confidence:** High  
**Affected components:** root `Cargo.toml`, `.github/workflows/ci.yml`, `apps/desktop/src-tauri/Cargo.lock`

### Evidence

The root workspace explicitly excludes:

```toml
exclude = ["apps/desktop/src-tauri", "crates/swarm-provider"]
```

The exact-head CI has a successful `Rust dependency audit` job using the root `cargo metadata --locked` and `rustsec/audit-check@v2.0.0`. The Desktop crate is packaged successfully on Linux, Windows, macOS Intel, and macOS ARM, but it has its own lockfile and is outside that root audit scope.

This is a dependency-vulnerability detection blind spot, not evidence that a vulnerable Tauri/Desktop dependency is presently exploitable.

### Required remediation

Add an explicit RustSec/advisory audit for `apps/desktop/src-tauri/Cargo.lock` (and any other separately locked excluded Rust package). Keep the root audit as well.

### Test/gate required to close

CI must fail when a known unignored advisory is introduced only into the Desktop lockfile.

---

# Positive controls already present

The review found substantial security work that should be preserved.

## Identity and signatures

- Application identity uses Ed25519 and `OsRng`.
- On Unix, application and transport private keys are created with mode `0600`.
- Key persistence uses temporary write, `sync_all`, rename, and parent-directory sync.
- `PeerIdentity` debug output does not serialize/display the secret key.
- Signature verification binds the supplied public key back to the claimed `PeerId` before verifying the signature.
- The network requires a signed `PeerHello` before ordinary replication requests become application events.

## Local Minecraft IPC

- Fabric control IPC binds loopback only.
- It generates a random 32-byte authentication token.
- The first control line must authenticate with the exact token.
- Peer IP is required to be loopback.
- Control lines are bounded (`16 KiB`) and the session applies timeouts.
- Debug formatting redacts the token.

## Filesystem and canonical storage

- Snapshot walking disables link following and rejects symlink entries.
- Snapshot paths are normalized/validated as portable relative paths.
- Restore checks every parent component, rejects symlinks, and writes through unique temporary files before publication.
- Blob contents are hash-verified during restore, including decompressed-length enforcement.
- Imported Minecraft worlds require a regular non-symlink `level.dat`, are snapshotted through the hardened storage path, and are published from staging.
- Server-mod JAR inspection has explicit total JAR/metadata size caps and canonical artifact hashing.

## Provider/runtime integrity

- Canonical provider artifacts are verified after download using SHA-1/SHA-256/SHA-512; MD5-only automatic acquisition is rejected.
- Provider filenames must be a single safe `.jar` basename.
- Managed runtime artifacts use hashes/checksums before use.
- External commands are generally built with `Command`/argv rather than shell command strings, preventing classic shell metacharacter injection.

## Desktop DOM/CSP

- Reviewed dynamic provider/world/error strings are written with `textContent` or DOM node properties rather than interpolated into HTML.
- Reviewed `innerHTML` uses are static application templates.
- Tauri CSP is local-first (`default-src 'self'`; script source restricted to self) and does not intentionally load a remote application frontend.

## CI evidence at audited SHA

Existing exact-head runs were green for:

- root Rust on Ubuntu, Windows, macOS;
- Desktop native package builds on Linux, Windows, macOS x86_64, macOS arm64;
- hostile network input remaining nonfatal;
- network handshake hardening;
- storage failure injection;
- live join replication;
- process-level host/migration/recovery tests;
- fuzz smoke for signed canonical record decoders;
- root RustSec dependency audit;
- Network Soak and Player Journey Live Acceptance workflows.

These are meaningful controls, but they do not cover every authorization/privacy boundary listed above.

---

# Explicit search review

The audit explicitly inspected/search-mapped the requested classes:

- `unsafe`: the meaningful production occurrence reviewed was the platform file-lock wrapper around OS locking; no attacker-controlled unsafe memory operation was identified.
- `Command`: runtime Java, curl, sidecars, `tar`, and platform folder-opening calls were inspected. Arguments are passed as argv rather than shell strings. The principal command-boundary finding is SEC-07-003, not shell metacharacter injection.
- shell execution: no production path was identified that constructs an attacker-controlled shell command string for `sh -c`, `cmd /C`, or equivalent.
- filesystem joins/untrusted paths: snapshot/import paths contain explicit symlink and portable-path controls; temporary diagnostics are weaker (SEC-07-004).
- URL handling/redirects: provider initial URLs use HTTPS restrictions and host checks in several paths, but redirect handling for secret-bearing CurseForge requests is unsafe (SEC-07-002).
- localhost listeners: Fabric bridge is loopback-bound and token-authenticated.
- Tauri permissions/IPC: privileged command surface and runtime path trust assessed in SEC-07-003.
- secret-bearing logs: no direct logging of application private keys or CurseForge key was found; curl argv remains a secret exposure channel.
- `unwrap`/`expect`: reviewed production hits were predominantly invariant-preserving constant conversions or test code. No confirmed attacker-controlled panic producing a reliable remote DoS was identified.

---

# Security tests missing / recommended

Priority order:

1. **Authorization matrix for every `WireRequest` variant.** For each request, test stranger, current member, banned member, removed member, current authority, and invited-but-not-yet-member where relevant.
2. **Private-world metadata regression test.** A valid authenticated ex-member that knows the world ID must receive no `WorldDescriptor`, `WorldStatus`, or `HostCapability` information.
3. **CurseForge cross-origin redirect test.** Use an HTTPS fixture and assert `x-api-key` never reaches another origin.
4. **Secret process-boundary test.** Ensure the CurseForge credential is not placed in child argv or logs.
5. **Tauri privilege-boundary test.** Standard UI flow must not choose an arbitrary executable path; test command/window scoping if capabilities are adopted.
6. **Temp-file symlink race tests.** Precreate symlinks/non-regular files and prove diagnostics/provider temp writes fail closed.
7. **Per-peer resource-exhaustion tests.** Authenticated malicious peers should be unable to monopolize request-response concurrency, disk reads, or repeated expensive world-status/capability computation. Current frame/collection limits are useful but are not a per-peer request-rate policy.
8. **Desktop RustSec gate.** Audit `apps/desktop/src-tauri/Cargo.lock` independently of the root lockfile.
9. **Frontend injection regression suite.** Feed malicious provider/world names, descriptions, tags, errors, and diagnostics containing HTML/script payloads and assert inert text rendering.
10. **Non-Unix key-permission test.** Prove Windows/macOS installed builds create identity files with intended user-only access rather than merely relying on environmental defaults.

---

# Items investigated but not raised as vulnerabilities

## Snapshot/import traversal

The snapshot and restore implementation explicitly rejects symlinks and unsafe relative paths, validates restore parents, bounds decompressed output by the manifest's declared size, and verifies blob hashes. Existing failure-injection tests include symlinked-parent restore cases. No arbitrary-filesystem-write defect was confirmed in canonical snapshot restore/import.

## Provider JAR substitution

Downloaded provider artifacts are verified against canonical strong hashes (SHA-1/256/512), and MD5-only automatic acquisition is rejected. This materially limits malicious redirect/download substitution. SEC-07-002 concerns the API credential, not successful execution of an unverified replacement JAR.

## Classic command injection

External commands use argument vectors. User/provider values are not concatenated into a shell command string. Arbitrary runtime path selection is documented separately as a frontend/native privilege-boundary issue.

## DOM injection

No direct exploitable DOM injection was identified in the reviewed primary desktop flows. Dynamic values are generally assigned via `textContent`; static HTML templates account for reviewed `innerHTML` usage. This lowers the current exploitability of SEC-07-003 but does not make native backend validation unnecessary.

## Fabric IPC authentication

The local Fabric bridge has a strong random bearer token, loopback peer enforcement, bounded input, and timeouts. No unauthenticated local control primitive was confirmed.

---

# Remediation order

1. **Fix SEC-07-001 first.** Add membership authorization to all world-scoped replication metadata endpoints and an exhaustive request authorization matrix.
2. **Fix SEC-07-002.** Move authenticated CurseForge HTTP to a client/redirect policy that never exposes the API key in argv or cross-origin redirects.
3. **Reduce Tauri frontend authority (SEC-07-003).** Make standard runtime setup backend-resolved and constrain privileged/advanced commands.
4. **Harden temp-file publication (SEC-07-004).** Use private-directory, exclusive/no-follow files.
5. **Extend dependency audit coverage (SEC-07-005).** Add Desktop lockfile RustSec.
6. Add hostile-rate/resource-exhaustion tests and keep existing fuzz/network soak gates.

---

# Final assessment

SwarmCraft's cryptographic identity, canonical artifact verification, snapshot path safety, local Fabric IPC authentication, and several fail-closed runtime/storage controls are materially stronger than a typical prototype. The security posture is nevertheless not acceptable as PASS because the network layer currently conflates “peer authenticated” with “authorized for this world” for several sensitive metadata calls.

Until SEC-07-001 is fixed and regression-tested, private-world membership/topology confidentiality is not enforced by the replication daemon.

**VERDICT: FAIL**
