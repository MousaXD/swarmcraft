# Auditor 6 - Package Providers and Supply Chain

Repository: `MousaXD/swarmcraft`

Audit branch: `audit/package-supply-chain`

Audited baseline: `354be3b1066428ecab6987590b7c7dbd80fe0870`

Audit scope: `crates/swarm-catalog`, `crates/swarm-provider`, the shared Modrinth provider implementation, the Desktop CurseForge provider, canonical modpack/provider provenance, provider-related Tauri commands, provider staging/download logic, and runtime provider reacquisition.

## Executive verdict

SwarmCraft has a strong exact-byte model once an artifact has been canonicalized: provider project/version identities are frozen, mutable provider URLs are excluded from canonical provenance, the runtime artifact bytes receive an independent domain-separated hash, required dependency closure is validated, provider downloads are rehashed, and runtime installation rechecks the signed mod ID/version/artifact hash.

The supply-chain boundary nevertheless fails this audit because the current Desktop CurseForge path lets provider-controlled filenames escape the intended staging directory, authenticated CurseForge API redirects can disclose the API key to another HTTPS origin, and the canonical model can label an MD5-only CurseForge artifact as `provider_download` even though runtime reacquisition deliberately refuses MD5-only automatic recovery. Provider API metadata bodies are also not consistently bounded.

**REPRODUCIBILITY VERDICT: FAIL**

**VERDICT: FAIL**

## Method and evidence limits

The live remote `main` SHA was verified before beginning and exactly matched the requested baseline. The audit branch was created from that SHA. Production code was not modified.

Source inspection used immutable GitHub content at the audited SHA. The local terminal connector was unavailable because of its conversation-identity guard, so this report does not claim a local reproduction run. At audit time, exact-head CI run `33576322543` was still in progress. Its Linux and Windows Desktop package jobs were already successful; the Linux job also reported successful frontend Desktop tests and Tauri bridge validation. These CI checks do not exercise the hostile-provider cases below.

External behavior relied on by Finding SC6-02 was checked against:

- curl manual, `--header`: https://curl.se/docs/manpage.html#-H
- curl known risks: https://curl.se/docs/knownrisks.html
- reqwest `v0.12.22` redirect implementation: https://github.com/seanmonstar/reqwest/blob/v0.12.22/src/redirect.rs

The repository pins/locks reqwest `0.12.22` for the relevant Desktop path.

## Provider trust-boundary diagram

```text
                              UNTRUSTED / EXTERNAL

  Mojang HTTPS                 Fabric Meta HTTPS
       |                              |
       v                              v
  +--------------------------------------------------+
  | swarm-catalog                                     |
  | HTTPS-only, no redirects, response-size bounds,  |
  | parser cardinality/string bounds                 |
  +--------------------------------------------------+
       |
       +--> URL-keyed local catalog cache
            fresh -> stale fallback with warning

  Modrinth API HTTPS
       |
       | exact project/version/file metadata
       v
  +--------------------------+       redirect can leave allowlisted host [SC6-05]
  | shared Modrinth provider |----------------------------------------------+
  +--------------------------+                                              |
       | exact SHA-1/SHA-512 locator                                        v
       v                                                             HTTPS target
  Modrinth CDN HTTPS
       |
       v
  temp file -> size + provider hash -> fsync -> publish
       |
       v
  canonicalize exact bytes

  CurseForge API HTTPS + machine-local API key
       |
       | x-api-key + cross-origin redirect allowed [SC6-02]
       v
  +---------------------------+
  | Desktop CurseForge client |
  +---------------------------+
       | exact project/file metadata
       | provider-controlled fileName [SC6-01]
       | HTTPS download URL, any host [SC6-05]
       v
  provider staging path assembled in frontend
       |
       v
  temp file -> declared-size bound -> SHA-1/MD5 verification -> fsync -> rename
       |
       v
  canonicalize exact bytes

                              TRUST TRANSITION

  CanonicalModpackV1
       - exact Minecraft + Fabric Loader versions
       - exact provider/project/version-file identity
       - provider hashes
       - exact file size/name
       - exact dependency targets
       - domain-separated exact runtime artifact hash
       - mutable URLs excluded
       |
       v
  signed RuntimeCompatibilityManifestV1
       |
       v
  runtime provider reacquisition
       |
       +--> recheck provider identity/hash/size
       +--> add_local_mod rechecks Fabric mod id + version + exact runtime hash
       |
       v
  executable server mod JAR
```

## Findings summary

| ID | Severity | Finding | Confidence |
| --- | --- | --- | --- |
| SC6-01 | CRITICAL | Provider-controlled CurseForge filename can escape Desktop staging and create a new `.jar` outside the provider directory | High |
| SC6-02 | HIGH | CurseForge API key can be sent to a different HTTPS origin after a redirect | High |
| SC6-03 | HIGH | MD5-only CurseForge artifacts can become canonical `provider_download` entries that runtime reacquisition cannot reproduce | High |
| SC6-04 | HIGH | Modrinth and CurseForge API metadata responses are not byte-bounded, enabling provider-driven memory/disk exhaustion | High |
| SC6-05 | MEDIUM | Redirect handling escapes provider host trust boundaries and creates an HTTPS SSRF/request primitive | Medium-High |

---

## SC6-01 - CurseForge filename traversal escapes provider staging

**Severity:** CRITICAL

**Affected code:**

- `apps/desktop/src/launcher-controller.js`
  - `prepareCurseForge`
- `apps/desktop/src-tauri/src/curseforge.rs`
  - `map_file`
  - `download_artifact`
  - `create_temp_artifact`
  - `publish_verified_artifact`
  - `curseforge_download`
- `apps/desktop/src-tauri/src/launcher_commands.rs`
  - `provider_staging_dir`

**Invariant:** Provider metadata must never choose a filesystem path outside the provider staging root. A provider filename must be data, not a path.

**Evidence:**

`map_file` takes `fileName` from CurseForge through `required_string`, which only requires a non-empty string. The frontend then uses that provider value directly in:

```text
${staging}/curseforge/${project_id}/${version_id}/${fileName}
```

and sends the resulting `destination` to the Tauri `curseforge_download` command.

The backend converts the supplied string to `PathBuf`. `download_artifact` requires only that the destination extension is `.jar` and that the final destination does not already exist. It does not require the destination to remain under the SwarmCraft provider staging root, and it does not require the destination basename to equal a safe one-component provider filename. `create_temp_artifact` creates the destination parent directories and `publish_verified_artifact` renames the verified temporary file to that destination.

A hostile provider response containing a name such as `../../../../chosen/target.jar` therefore turns the intended staging path into a traversal path. The exact number of `..` components can be chosen for the installation location. On Windows, separator variants must also be considered.

The `destination_exists` guard prevents replacing an existing target, and the extension check restricts the final name to `.jar`, but those constraints do not restore the intended filesystem boundary. The provider can still cause creation of a new provider-controlled JAR at a writable path outside staging.

**Attack/failure scenario:**

1. CurseForge metadata for an otherwise selected compatible file supplies a traversal-bearing `fileName` ending in `.jar`.
2. Desktop resolves the file and constructs the destination by string concatenation.
3. Tauri receives the escaped path.
4. The downloader creates parent directories, streams provider bytes, verifies provider metadata, fsyncs the file, and renames it to the escaped destination.
5. The write occurs outside `provider-staging` before canonicalization gets a chance to reject or constrain the path.

This is arbitrary filesystem write of a new `.jar` within the user's writable locations, controlled through a remote supply-chain boundary. It is therefore CRITICAL under the project's arbitrary-filesystem-access severity rule.

**Existing test coverage:**

`launcher_commands.rs` has a `staging_path_is_not_player_supplied` test, but it only checks a locally constructed staging suffix. It does not test provider-controlled filenames, containment, `..`, absolute paths, drive prefixes, UNC paths, or separator variants.

**Missing test:**

A provider fixture must exercise filenames including `../x.jar`, `../../x.jar`, absolute Unix paths, Windows drive paths, UNC paths, forward/backslash mixtures, `.`/`..` components, and encoded/normalized variants. The backend should prove that the final resolved destination is a child of the server-owned staging directory on Linux, macOS, and Windows.

**Recommended remediation:**

1. Validate CurseForge `fileName` as exactly one normal path component before it is exposed to the frontend.
2. Repeat the validation in the Tauri backend. Do not rely on frontend validation.
3. Do not accept an arbitrary destination path for provider downloads. Accept an opaque staging/session identifier plus exact provider identity, then construct the full path server-side.
4. Reject absolute paths, root/prefix components, `.`/`..`, path separators for the current platform and cross-platform separator forms.
5. Enforce staging containment after normalized path construction.
6. Keep the final runtime copy under `server_mods::add_local_mod`, which already derives its own canonical filename.

**Confidence:** High.

---

## SC6-02 - CurseForge API key follows cross-origin redirects

**Severity:** HIGH

**Affected code:**

- `apps/desktop/src-tauri/src/curseforge.rs`
  - `CurseForgeClient::from_environment`
  - `get_json`
  - `post_json`
- `crates/swarm-cli/src/provider_runtime.rs`
  - `curseforge_json`

**Invariant:** `SWARMCRAFT_CURSEFORGE_API_KEY` must only be transmitted to the intended CurseForge API origin.

**Evidence:**

The Desktop CurseForge reqwest client installs a custom redirect policy that permits up to five redirects as long as the next URL remains HTTPS. It does not constrain the redirect host to `api.curseforge.com`. API requests attach the key as the custom `x-api-key` header.

The audited Desktop dependency resolves to reqwest `0.12.22`. Its redirect code strips a limited set of known sensitive headers on cross-origin redirects, including `Authorization`, `Cookie`, `Proxy-Authorization`, and related headers. It does not strip arbitrary `x-api-key` headers.

The runtime reacquisition path has the same problem through a different HTTP stack. `curseforge_json` invokes curl with both `-L` and a custom `-H "x-api-key: ..."`. The curl manual explicitly warns that headers set with `--header` are included in requests after redirects and may be sent to a different host. curl's special cross-origin credential handling does not make an arbitrary `x-api-key` header safe.

**Attack/failure scenario:**

1. The CurseForge API endpoint, or an attacker controlling its effective response path, returns a 30x to `https://attacker.example/...`.
2. Desktop reqwest follows the redirect and retains the custom `x-api-key` header.
3. The runtime curl path also follows the redirect with the custom header.
4. The attacker receives the machine-local CurseForge API credential.

This is a direct credential-confidentiality failure at the provider trust boundary.

**Existing test coverage:** No redirect-origin test was found for the authenticated CurseForge API paths.

**Missing test:** Run a local two-origin HTTPS fixture where origin A returns a 30x to origin B. Assert that authenticated API operations either reject the redirect or that origin B never receives `x-api-key`. Cover GET and POST plus the runtime curl path.

**Recommended remediation:**

- For authenticated CurseForge API calls, disable redirects or allow only the exact expected API origin, including scheme, host, and expected port.
- Separate authenticated API clients from unauthenticated artifact-download clients.
- For the curl runtime path, avoid `-L` on authenticated API calls. If provider API redirects are genuinely required, validate `Location` manually and reissue only to the same approved origin.
- Keep artifact-download redirects under a distinct policy that never carries the API key.

**Confidence:** High.

---

## SC6-03 - Canonical `provider_download` can be unreproducible for MD5-only CurseForge files

**Severity:** HIGH

**Affected code:**

- `apps/desktop/src-tauri/src/curseforge.rs`
  - `provider_hashes`
  - `verify_provider_hashes`
  - `download_artifact`
- `apps/desktop/src/launcher-controller.js`
  - `providerHashes`
  - `canonicalPackageFromDownloaded`
- `apps/desktop/src-tauri/src/canonical_commands.rs`
  - `verify_provider_hashes`
  - `canonicalize_package`
- `crates/swarm-protocol/src/canonical_modpack.rs`
  - `CanonicalHashAlgorithmV1`
  - `validate_provider_artifact`
- `crates/swarm-cli/src/provider_runtime.rs`
  - `verify_canonical_hashes`
  - `acquire_curseforge`

**Invariant:** If canonical provenance says `ProviderDownload`, another clean peer must be able to reacquire the exact artifact when the provider still exposes that exact file. If automatic proof is insufficient, the canonical retrieval state must be `ManualRequired` instead.

**Evidence:**

CurseForge metadata maps SHA-1 and MD5 provider hashes. The initial Desktop downloader can verify MD5. `canonicalPackageFromDownloaded` marks successfully downloaded provider files as `provider_download` and passes the provider hashes into canonicalization. `canonical_commands::verify_provider_hashes` supports MD5 and accepts an MD5-only list. `CanonicalModpackV1::validate` only requires that a provider-backed artifact have at least one well-formed provider hash, and MD5 is a valid enum value.

Runtime reacquisition deliberately has a stronger rule. `verify_canonical_hashes` computes SHA-1/SHA-256/SHA-512, ignores MD5, and fails if no strong algorithm was verified. Its explicit error states that MD5-only artifacts must be supplied manually.

The codebase therefore admits this state:

```text
canonical retrieval = provider_download
canonical provider hashes = [md5]
initial world creation = succeeds
clean-peer runtime reacquisition = fails and demands manual remediation
```

The exact runtime artifact hash still prevents accepting the wrong JAR. This finding is a reproducibility failure, not a silent-integrity failure. The system fails closed, but the canonical retrieval contract overpromises what can actually be reproduced.

**Existing test coverage:** `provider_runtime.rs` has a test proving that MD5-only provider identity requires manual runtime remediation. No corresponding canonical validation test prevents `ProviderDownload` from being encoded with MD5-only provenance.

**Missing test:** Construct a full canonical CurseForge package with only MD5, `ProviderDownload`, and exact runtime bytes. The canonicalization layer must either reject it, downgrade it to `ManualRequired`, or prove that runtime reacquisition can use the exact canonical runtime hash safely.

**Recommended remediation:** Choose one coherent contract:

1. Require at least SHA-1/SHA-256/SHA-512 for `CanonicalRetrievalV1::ProviderDownload`; classify MD5-only files as `ManualRequired`, or
2. For CurseForge exact project/file IDs, permit retrieval but validate the downloaded bytes against the signed canonical runtime artifact hash before publication, making the runtime hash the final proof even when provider metadata is only MD5.

Whichever design is selected, make canonical validation and runtime reacquisition enforce the same rule.

**Confidence:** High.

---

## SC6-04 - Provider API metadata responses are unbounded

**Severity:** HIGH

**Affected code:**

- `crates/swarm-cli/src/package_provider/modrinth.rs`
  - `CurlTransport::get`
  - `ModrinthClient::request_json`
- `apps/desktop/src-tauri/src/curseforge.rs`
  - `parse_json_response`
- `crates/swarm-cli/src/provider_runtime.rs`
  - `curseforge_json`

**Invariant:** A hostile or malfunctioning provider must not be able to force unbounded disk or memory consumption through metadata responses.

**Evidence:**

The catalog subsystem demonstrates the intended control: Mojang and Fabric responses have explicit byte limits, bounded reads, maximum entry counts, and bounded provider strings.

The package-provider paths do not consistently apply the same discipline:

- Modrinth API metadata is written by curl to a temporary body file and then loaded with `fs::read` without a body-size limit. Response headers are likewise read as a whole string.
- Desktop CurseForge parses `response.json::<Value>()` without an explicit response-body byte limit.
- Runtime CurseForge writes the response to a temporary path and then reads the entire file with `fs::read` without a metadata limit.

Timeouts limit duration, not bytes. A provider capable of returning a very large body can consume disk and then memory in the curl paths, or memory directly in the reqwest JSON path.

**Attack/failure scenario:** A hostile or compromised provider endpoint returns a multi-gigabyte successful JSON-like response within the configured timeout. SwarmCraft stores/buffers it before JSON parsing and can exhaust local memory or disk.

**Existing test coverage:** Catalog oversized-response tests exist; equivalent oversized package-provider metadata tests were not found.

**Missing test:** Provider fixtures should return Content-Length values and streamed bodies beyond the metadata budget. Each API call must fail with a structured `response_too_large` style error before allocating or writing beyond the configured cap.

**Recommended remediation:**

- Add explicit metadata limits for Modrinth and CurseForge, preferably a low single-digit MiB budget per response.
- Reject oversized `Content-Length` early.
- Stream/take at most `limit + 1` bytes and abort once exceeded.
- Add cardinality and string-size checks for provider arrays and text fields after parsing.
- Ensure temporary response files are removed on every failure path.

**Confidence:** High.

---

## SC6-05 - Redirects leave provider host trust boundaries

**Severity:** MEDIUM

**Affected code:**

- `crates/swarm-cli/src/package_provider/modrinth.rs`
  - `CurlTransport::get`
  - `CurlTransport::download`
  - `trusted_https`
- `apps/desktop/src-tauri/src/curseforge.rs`
  - `CurseForgeClient::from_environment`
  - `validate_download_url`
- `crates/swarm-cli/src/provider_runtime.rs`
  - `curl_download`

**Invariant:** Provider-controlled URLs and redirects must not turn the client into a request primitive toward arbitrary HTTPS services outside the intended provider/CDN boundary.

**Evidence:**

Modrinth validates the initial API host as `api.modrinth.com` and the initial artifact host as `cdn.modrinth.com`, which is a good control. Both curl operations then use `-L`; the code does not validate each redirect destination host. `--proto =https` prevents protocol downgrade but does not keep the request on the approved host.

CurseForge's reqwest redirect policy similarly checks only HTTPS. `validate_download_url` accepts any parseable HTTPS host. Runtime CurseForge download uses curl `-L` with an HTTPS-only protocol rule but no destination-host policy.

Exact size/hash checks mean an off-origin artifact is unlikely to be accepted unless it has the canonical bytes, so this is not an artifact-substitution finding. It is a network trust-boundary finding: a hostile provider can cause the client to issue HTTPS requests to another reachable service, including local/private network HTTPS endpoints. Finding SC6-02 is the more severe authenticated variant.

**Existing test coverage:** HTTPS scheme checks exist. No cross-host redirect rejection test was found for provider API or artifact paths.

**Missing test:** Use a two-origin redirect fixture and prove which redirect hosts are explicitly allowed for Modrinth API, Modrinth CDN, CurseForge API, and CurseForge artifact retrieval. Include private-address targets.

**Recommended remediation:**

- Define redirect policy separately for each provider API and download surface.
- Keep authenticated API requests same-origin.
- For artifact redirects, use an explicit provider/CDN allow policy where practical. If provider CDNs genuinely require multiple dynamic hosts, add a narrowly documented policy and reject loopback/private/link-local destinations after DNS resolution.
- Revalidate the destination at every redirect hop rather than only the initial URL.

**Confidence:** Medium-High because the request primitive is clear in code, while the exact set of legitimate provider CDN redirect hosts is an external operational constraint.

---

## Positive controls already present

### Catalogs

`crates/swarm-catalog` is notably defensive:

- Mojang and Fabric sources are fixed HTTPS URLs.
- reqwest is configured HTTPS-only.
- redirects are disabled.
- connect and total timeouts are set.
- network and cache body sizes are bounded.
- provider entry counts and important string lengths are bounded.
- duplicate version IDs are rejected.
- Fabric loader validation is scoped to the exact requested Minecraft version.
- cache files are keyed by the source URL hash and the stored source URL must match before reuse.
- stale cache fallback is surfaced explicitly through `CatalogOrigin::StaleCache` plus a warning rather than masquerading as fresh network data.

### Modrinth

The exact artifact path has good integrity controls:

- initial API/CDN hosts are allowlisted and HTTPS-only;
- exact project and version IDs are checked;
- an exact SHA-1 or SHA-512 locator is required;
- compatible Minecraft/Fabric/environment filters are revalidated;
- required dependency cycles and conflicting project versions are rejected;
- download size has a configured and absolute upper bound;
- artifact filenames are checked as basenames;
- bytes are written to a temporary file, exact size and provider hashes are checked, the file is fsynced, and publication uses rename/rollback logic.

### CurseForge

- the API credential is machine-local and is not hardcoded;
- missing credentials fail as configuration-required rather than silently degrading to scraping;
- supported search pagination and dependency graph size are bounded;
- selected files are checked for exact Minecraft and Fabric tags;
- required and optional dependencies remain distinct;
- provider-restricted downloads surface an explicit manual-artifact-required state;
- artifact downloads are streamed under a declared-size and 512 MiB absolute cap;
- temporary files use `create_new`, are fsynced, and are cleaned unless publication succeeds;
- provider SHA-1/MD5 metadata is checked when present.

### Canonical provenance and runtime

- canonical records freeze provider/project/version-file identity, filename, file size, hashes, retrieval mode, exact dependency targets, and exact runtime bytes;
- mutable provider download URLs are intentionally excluded from canonical provenance;
- package ordering is normalized, so provider response order does not change the compatibility fingerprint;
- ambiguous versions such as `latest` and range syntax are rejected;
- required dependency closure and exact incompatible selections are validated;
- canonicalization re-reads exact artifact bytes, verifies provider hashes, and computes the domain-separated runtime artifact hash;
- runtime reacquisition rechecks provider identity, filename, size, and canonical hashes;
- `server_mods::add_local_mod` independently inspects the JAR and rejects a wrong mod ID/version/runtime artifact hash before installation.

These controls are why no silent provider substitution finding was identified after canonicalization: a changed provider file is expected to fail closed.

## Reproducibility assessment

| Property | Assessment | Notes |
| --- | --- | --- |
| Exact Minecraft version selection | PASS | Source-backed catalog with exact token validation |
| Exact Fabric Loader selection | PASS | Compatibility checked against exact Minecraft version |
| Exact Modrinth project/version/file identity | PASS | Project/version plus strong provider file hash locator |
| Exact CurseForge project/file identity | PASS | Numeric project/file IDs frozen and revalidated |
| Exact artifact bytes frozen | PASS | Domain-separated runtime artifact hash is canonical |
| Mutable provider URL excluded from canonical state | PASS | Canonical provider source stores provenance, not URL |
| Required dependency closure frozen | PASS | Exact required targets validated |
| Provider outage fails closed | PASS | No silent alternate artifact substitution |
| Restricted CurseForge file fails closed | PASS | Explicit manual artifact path |
| Cache poisoning across catalog URLs | PASS | URL-keyed cache plus stored source URL match |
| Automatic reacquisition contract | **FAIL** | MD5-only `ProviderDownload` can be canonical but unreacquirable |
| Provider filename filesystem containment | **FAIL** | Desktop CurseForge filename can escape staging |
| Credential containment across redirects | **FAIL** | `x-api-key` can cross origins |
| Malicious metadata resource bounds | **FAIL** | Package-provider API bodies are not explicitly bounded |

## Provider/cache/offline behavior

- Mojang/Fabric catalog network failure can fall back to a previously parsed stale official cache and marks that fact in the response. This supports offline selection without pretending the cache is fresh.
- Modrinth/CurseForge provider selection and exact missing-artifact reacquisition do not silently choose substitutes on outage. They fail closed.
- Already installed exact runtime artifacts are reused when they match the signed runtime requirement, avoiding unnecessary provider access.
- No general exact-artifact content-addressed provider cache was identified in this scope. Reacquisition therefore still depends on either the already installed exact JAR, provider availability, or the explicit manual path.

## Additional trust assumptions

Mojang and Fabric catalog authenticity currently rests on normal HTTPS PKI and the fixed official endpoints. The catalog itself does not verify a provider signature over the version lists. That is an external trust-root assumption rather than a demonstrated substitution defect in this audit.

A provider can always serve malicious bytes during the *initial* selection and publish matching provider hash metadata. SwarmCraft cannot independently prove publisher intent without a provider or author signature scheme. The important control SwarmCraft does provide is immutability after selection: the exact bytes are locally hashed and frozen into canonical state, so a later provider change cannot silently replace them.

## Remediation order

1. **SC6-01:** move all CurseForge destination construction into the backend and enforce safe-basename plus staging containment before any filesystem creation.
2. **SC6-02:** make authenticated CurseForge API requests same-origin/no-redirect so the API key cannot cross origins.
3. **SC6-03:** align canonical `ProviderDownload` validation with runtime reacquisition proof requirements.
4. **SC6-04:** add explicit metadata response byte/cardinality/string limits to Modrinth and CurseForge.
5. **SC6-05:** define per-provider redirect destination policies, including private-address handling.

## Tests required to close this audit

- Cross-platform hostile CurseForge `fileName` containment tests through the actual Tauri download command.
- Two-origin authenticated redirect tests proving `x-api-key` never reaches the second origin in Desktop and runtime paths.
- End-to-end canonicalization/reacquisition test for a CurseForge MD5-only fixture.
- Oversized Modrinth API response tests.
- Oversized CurseForge API response tests for both reqwest Desktop and curl runtime clients.
- Redirect-hop allowlist/private-network tests for Modrinth and CurseForge.
- A clean-peer acceptance test that creates a provider-backed canonical world on Player A, deletes provider artifacts on Player B, reacquires them, and proves the final runtime hash and compatibility fingerprint remain unchanged.

Until those defects are remediated and the tests above are green on the exact fix SHA, this supply-chain audit remains failed.
