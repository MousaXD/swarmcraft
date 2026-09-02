# Agent 4 transport admission milestone failure

Seed: 6419b98d7d4d2d1d4f1d9a1d2e7a3c90703fb0bd

## implementation/agent4-transport-admission-apply.log
```text
```

## implementation/agent4-transport-admission-check.log
```text
    Updating crates.io index
    Updating git repository `https://github.com/libp2p/rust-libp2p`
 Downloading crates ...
  Downloaded icu_locale_core v2.3.0
  Downloaded match-lookup v0.1.3
  Downloaded icu_collections v2.3.0
  Downloaded aead v0.5.2
  Downloaded matchers v0.2.0
  Downloaded option-ext v0.2.0
  Downloaded same-file v1.0.6
  Downloaded nohash-hasher v0.2.0
  Downloaded num-conv v0.2.2
  Downloaded async-trait v0.1.92
  Downloaded atomic-waker v1.1.2
  Downloaded asn1-rs-impl v0.2.0
  Downloaded asynchronous-codec v0.7.0
  Downloaded opaque-debug v0.3.1
  Downloaded cfg_aliases v0.2.2
  Downloaded percent-encoding v2.3.2
  Downloaded hyper-util v0.1.20
  Downloaded itoa v1.0.18
  Downloaded lazy_static v1.5.0
  Downloaded prometheus-client-derive-encode v0.5.0
  Downloaded potential_utf v0.1.6
  Downloaded idna_adapter v1.2.2
  Downloaded rand_pcg v0.10.2
  Downloaded rusticata-macros v4.1.0
  Downloaded if-watch v3.2.2
  Downloaded inout v0.1.4
  Downloaded base64ct v1.8.3
  Downloaded cfg-if v1.0.4
  Downloaded is_terminal_polyfill v1.70.2
  Downloaded powerfmt v0.2.0
  Downloaded rustc_version v0.4.1
  Downloaded block-buffer v0.12.1
  Downloaded rustc-hash v2.1.3
  Downloaded block-buffer v0.10.4
  Downloaded multiaddr v0.19.0
  Downloaded multihash v0.19.5
  Downloaded netlink-proto v0.12.2
  Downloaded netlink-sys v0.8.8
  Downloaded num-integer v0.1.47
  Downloaded oid-registry v0.8.1
  Downloaded pkg-config v0.3.34
  Downloaded nu-ansi-term v0.50.3
  Downloaded pem v3.0.6
  Downloaded anstyle-query v1.1.5
  Downloaded scopeguard v1.2.0
  Downloaded multibase v0.9.3
  Downloaded igd-next v0.17.1
  Downloaded polyval v0.6.2
  Downloaded prost-derive v0.14.4
  Downloaded bs58 v0.5.1
  Downloaded ipnet v2.12.1
  Downloaded lock_api v0.4.14
  Downloaded lru-slab v0.1.2
  Downloaded paste v1.0.15
  Downloaded pin-project-internal v1.1.13
  Downloaded pin-project-lite v0.2.17
  Downloaded pkcs8 v0.10.2
  Downloaded quinn-udp v0.5.15
  Downloaded quote v1.0.47
  Downloaded resolv-conf v0.7.6
  Downloaded rustversion v1.0.23
  Downloaded zstd-safe v7.2.4
  Downloaded hash32 v0.2.1
  Downloaded prost v0.14.4
  Downloaded stable_deref_trait v1.2.1
  Downloaded colorchoice v1.0.5
  Downloaded curve25519-dalek-derive v0.1.1
  Downloaded form_urlencoded v1.2.2
  Downloaded jobserver v0.1.35
  Downloaded libp2p-identity v0.3.0
  Downloaded litemap v0.8.3
  Downloaded netlink-packet-core v0.8.2
  Downloaded once_cell v1.21.4
  Downloaded parking_lot v0.12.5
  Downloaded parking_lot_core v0.9.12
  Downloaded poly1305 v0.8.0
  Downloaded postcard v1.1.3
  Downloaded proc-macro2 v1.0.107
  Downloaded rand_core v0.6.4
  Downloaded rand_core v0.10.1
  Downloaded rustls-pki-types v1.15.1
  Downloaded semver v1.0.28
  Downloaded tinyvec_macros v0.1.1
  Downloaded anyhow v1.0.104
  Downloaded clap_lex v1.1.0
  Downloaded data-encoding-macro-internal v0.1.19
  Downloaded hyper v1.11.1
  Downloaded icu_provider v2.3.1
  Downloaded js-sys v0.3.85
  Downloaded num-traits v0.2.19
  Downloaded pin-project v1.1.13
  Downloaded prefix-trie v0.8.4
  Downloaded rcgen v0.13.2
  Downloaded rtnetlink v0.20.0
  Downloaded sharded-slab v0.1.7
  Downloaded thiserror v2.0.20
  Downloaded time-macros v0.2.32
  Downloaded zerofrom-derive v0.1.7
  Downloaded blake2 v0.10.6
  Downloaded byteorder v1.5.0
  Downloaded cbor4ii v1.2.2
  Downloaded cipher v0.4.4
  Downloaded crypto-common v0.2.2
  Downloaded dirs-sys v0.4.1
  Downloaded embedded-io v0.4.0
  Downloaded errno v0.3.14
  Downloaded ghash v0.5.1
  Downloaded icu_normalizer_data v2.3.0
  Downloaded icu_properties v2.3.0
  Downloaded indexmap v2.14.1
  Downloaded log v0.4.34
  Downloaded memchr v2.8.3
  Downloaded minimal-lexical v0.2.1
  Downloaded mio v1.2.2
  Downloaded nom v7.1.3
  Downloaded num-bigint v0.4.8
  Downloaded prometheus-client v0.25.0
  Downloaded quinn v0.11.11
  Downloaded rand v0.10.2
  Downloaded rustls-webpki v0.103.15
  Downloaded tracing-attributes v0.1.31
  Downloaded uint v0.10.1
  Downloaded writeable v0.6.4
  Downloaded yasna v0.5.2
  Downloaded arrayref v0.3.9
  Downloaded arrayvec v0.7.8
  Downloaded asn1-rs-derive v0.6.0
  Downloaded cpufeatures v0.2.17
  Downloaded data-encoding-macro v0.1.21
  Downloaded directories v5.0.1
  Downloaded embedded-io v0.6.1
  Downloaded equivalent v1.0.2
  Downloaded fnv v1.0.7
  Downloaded fs2 v0.4.3
  Downloaded futures-bounded v0.3.0
  Downloaded futures-core v0.3.34
  Downloaded futures-macro v0.3.34
  Downloaded futures-sink v0.3.34
  Downloaded getrandom v0.2.17
  Downloaded getrandom v0.3.4
  Downloaded hashbrown v0.17.1
  Downloaded icu_properties_data v2.3.0
  Downloaded idna v1.1.0
  Downloaded itertools v0.14.0
  Downloaded libm v0.2.16
  Downloaded serde_core v1.0.229
  Downloaded serde_derive v1.0.229
  Downloaded spin v0.9.9
  Downloaded spki v0.7.3
  Downloaded thiserror-impl v1.0.69
  Downloaded time-core v0.1.9
  Downloaded tinystr v0.8.4
  Downloaded try-lock v0.2.5
  Downloaded typenum v1.20.1
  Downloaded x25519-dalek v3.0.0
  Downloaded aho-corasick v1.1.5
  Downloaded asn1-rs v0.7.2
  Downloaded dunce v1.0.5
  Downloaded moka v0.12.16
  Downloaded netlink-packet-route v0.28.0
  Downloaded portable-atomic v1.15.0
  Downloaded tinyvec v1.12.0
  Downloaded unsigned-varint v0.8.0
  Downloaded untrusted v0.9.0
  Downloaded utf8_iter v1.0.4
  Downloaded version_check v0.9.5
  Downloaded want v0.3.1
  Downloaded quinn-proto v0.11.17
  Downloaded unicode-ident v1.0.24
  Downloaded universal-hash v0.5.1
  Downloaded walkdir v2.5.0
  Downloaded cpufeatures v0.3.1
  Downloaded crunchy v0.2.4
  Downloaded base-x v0.2.11
  Downloaded base256emoji v1.0.2
  Downloaded base45 v3.2.0
  Downloaded static_assertions v1.1.0
  Downloaded url v2.5.8
  Downloaded wasm-bindgen v0.2.108
  Downloaded cmake v0.1.58
  Downloaded either v1.18.0
  Downloaded futures-io v0.3.34
  Downloaded futures-task v0.3.34
  Downloaded hashlink v0.12.1
  Downloaded nix v0.30.1
  Downloaded regex-syntax v0.8.11
  Downloaded rustls v0.23.43
  Downloaded send_wrapper v0.4.0
  Downloaded signature v3.0.0
  Downloaded syn v2.0.119
  Downloaded syn v3.0.4
  Downloaded chacha20 v0.10.2
  Downloaded cobs v0.3.0
  Downloaded data-encoding v2.11.1
  Downloaded digest v0.10.7
  Downloaded rustix v1.1.4
  Downloaded serde_json v1.0.151
  Downloaded thiserror-impl v2.0.20
  Downloaded thread_local v1.1.10
  Downloaded tracing v0.1.44
  Downloaded icu_normalizer v2.3.0
  Downloaded regex-automata v0.4.18
  Downloaded tokio-macros v2.7.2
  Downloaded tower-service v0.3.3
  Downloaded tracing-log v0.2.0
  Downloaded utf8parse v0.2.2
  Downloaded deranged v0.5.8
  Downloaded digest v0.11.3
  Downloaded ed25519 v3.0.0
  Downloaded fastrand v2.5.0
  Downloaded foldhash v0.2.0
  Downloaded futures-executor v0.3.34
  Downloaded futures-rustls v0.26.0
  Downloaded time v0.3.55
  Downloaded tracing-core v0.1.36
  Downloaded wasm-bindgen-macro-support v0.2.108
  Downloaded wasm-bindgen-shared v0.2.108
  Downloaded web-time v1.1.0
  Downloaded xml-rs v0.8.29
  Downloaded autocfg v1.5.1
  Downloaded const-str v0.4.3
  Downloaded constant_time_eq v0.4.2
  Downloaded critical-section v1.2.0
  Downloaded crypto-common v0.1.7
  Downloaded dtoa v1.0.11
  Downloaded fastbloom v0.17.0
  Downloaded find-msvc-tools v0.1.11
  Downloaded fs_extra v1.3.0
  Downloaded futures-timer v3.0.3
  Downloaded generic-array v0.14.7
  Downloaded gloo-timers v0.2.6
  Downloaded tokio-util v0.7.19
  Downloaded x509-parser v0.18.1
  Downloaded clap_derive v4.6.4
  Downloaded const-oid v0.9.6
  Downloaded ctr v0.9.2
  Downloaded sha1 v0.10.7
  Downloaded sha2 v0.10.9
  Downloaded shlex v2.0.1
  Downloaded signal-hook-registry v1.4.8
  Downloaded signature v2.2.0
  Downloaded siphasher v1.0.3
  Downloaded slab v0.4.12
  Downloaded smallvec v1.15.2
  Downloaded tracing-subscriber v0.3.23
  Downloaded chacha20 v0.9.1
  Downloaded libc v0.2.189
  Downloaded sha2 v0.11.0
  Downloaded snow v0.10.0
  Downloaded bytes v1.12.1
  Downloaded wasm-bindgen-macro v0.2.108
  Downloaded zeroize v1.9.0
  Downloaded zstd-sys v2.0.16+zstd.1.5.7
  Downloaded heck v0.5.0
  Downloaded yasna v0.6.0
  Downloaded yoke v0.8.3
  Downloaded cmov v0.5.4
  Downloaded ctutils v0.4.2
  Downloaded displaydoc v0.2.7
  Downloaded ed25519 v2.2.3
  Downloaded futures-channel v0.3.34
  Downloaded zerotrie v0.2.5
  Downloaded clap v4.6.6
  Downloaded futures v0.3.34
  Downloaded ring v0.17.14
  Downloaded tokio v1.53.1
  Downloaded socket2 v0.6.5
  Downloaded subtle v2.6.1
  Downloaded strsim v0.11.1
  Downloaded synstructure v0.13.2
  Downloaded tagptr v0.2.0
  Downloaded tempfile v3.27.0
  Downloaded thiserror v1.0.69
  Downloaded bitflags v2.13.1
  Downloaded zmij v1.0.23
  Downloaded anstyle v1.0.14
  Downloaded yoke-derive v0.8.2
  Downloaded zerofrom v0.1.8
  Downloaded anstream v1.0.0
  Downloaded anstyle-parse v1.0.0
  Downloaded zstd v0.13.3
  Downloaded zerovec-derive v0.11.6
  Downloaded zerovec v0.11.8
  Downloaded aes-gcm v0.10.3
  Downloaded crossbeam-utils v0.8.22
  Downloaded getrandom v0.4.3
  Downloaded http-body v1.1.0
  Downloaded yamux v0.14.0
  Downloaded const-oid v0.10.2
  Downloaded linux-raw-sys v0.12.1
  Downloaded chacha20poly1305 v0.10.1
  Downloaded crossbeam-epoch v0.9.20
  Downloaded der-parser v10.0.0
  Downloaded xmltree v0.10.3
  Downloaded hex v0.4.3
  Downloaded uuid v1.26.0
  Downloaded bumpalo v3.20.3
  Downloaded hybrid-array v0.4.14
  Downloaded http-body-util v0.1.5
  Downloaded cc v1.4.4
  Downloaded attohttpc v0.30.1
  Downloaded crossbeam-channel v0.5.16
  Downloaded der v0.7.10
  Downloaded httparse v1.10.1
  Downloaded serde v1.0.229
  Downloaded hmac v0.13.0
  Downloaded base64 v0.22.1
  Downloaded ed25519-dalek v3.0.0
  Downloaded http v1.5.0
  Downloaded ed25519-dalek v2.2.0
  Downloaded heapless v0.7.17
  Downloaded hickory-resolver v0.26.1
  Downloaded aes v0.8.4
  Downloaded clap_builder v4.6.6
  Downloaded futures-util v0.3.34
  Downloaded blake3 v1.8.7
  Downloaded hickory-net v0.26.1
  Downloaded h2 v0.4.19
  Downloaded hkdf v0.13.0
  Downloaded aws-lc-rs v1.18.0
  Downloaded curve25519-dalek v4.1.3
  Downloaded hickory-proto v0.26.1
  Downloaded curve25519-dalek v5.0.0
  Downloaded aws-lc-sys v0.44.0
   Compiling proc-macro2 v1.0.107
   Compiling quote v1.0.47
   Compiling unicode-ident v1.0.24
    Checking cfg-if v1.0.4
   Compiling libc v0.2.189
    Checking pin-project-lite v0.2.17
   Compiling semver v1.0.28
   Compiling thiserror v2.0.20
    Checking stable_deref_trait v1.2.1
   Compiling rustc_version v0.4.1
    Checking typenum v1.20.1
    Checking memchr v2.8.3
   Compiling syn v3.0.4
   Compiling syn v2.0.119
   Compiling portable-atomic v1.15.0
   Compiling serde_core v1.0.229
    Checking log v0.4.34
    Checking cpufeatures v0.3.1
   Compiling jobserver v0.1.35
    Checking critical-section v1.2.0
   Compiling shlex v2.0.1
   Compiling find-msvc-tools v0.1.11
    Checking once_cell v1.21.4
   Compiling cc v1.4.4
   Compiling synstructure v0.13.2
    Checking subtle v2.6.1
   Compiling serde v1.0.229
    Checking futures-io v0.3.34
    Checking slab v0.4.12
    Checking bytes v1.12.1
    Checking futures-core v0.3.34
    Checking zeroize v1.9.0
    Checking futures-sink v0.3.34
    Checking futures-channel v0.3.34
   Compiling thiserror-impl v2.0.20
   Compiling displaydoc v0.2.7
   Compiling serde_derive v1.0.229
   Compiling zerofrom-derive v0.1.7
   Compiling yoke-derive v0.8.2
   Compiling zerovec-derive v0.11.6
    Checking zerofrom v0.1.8
   Compiling futures-macro v0.3.34
    Checking yoke v0.8.3
    Checking rand_core v0.10.1
    Checking futures-task v0.3.34
    Checking smallvec v1.15.2
    Checking futures-util v0.3.34
    Checking zerovec v0.11.8
    Checking tracing-core v0.1.36
    Checking scopeguard v1.2.0
   Compiling getrandom v0.4.3
    Checking lock_api v0.4.14
   Compiling tracing-attributes v0.1.31
    Checking tinystr v0.8.4
    Checking hybrid-array v0.4.14
    Checking writeable v0.6.4
    Checking litemap v0.8.3
    Checking potential_utf v0.1.6
    Checking icu_locale_core v2.3.0
    Checking zerotrie v0.2.5
    Checking tracing v0.1.44
    Checking cmov v0.5.4
    Checking utf8_iter v1.0.4
   Compiling icu_properties_data v2.3.0
   Compiling icu_normalizer_data v2.3.0
    Checking byteorder v1.5.0
    Checking ctutils v0.4.2
    Checking icu_collections v2.3.0
    Checking crypto-common v0.2.2
    Checking block-buffer v0.12.1
    Checking const-oid v0.10.2
    Checking icu_provider v2.3.1
    Checking chacha20 v0.10.2
   Compiling curve25519-dalek-derive v0.1.1
   Compiling anyhow v1.0.104
    Checking rand v0.10.2
    Checking icu_properties v2.3.0
    Checking digest v0.11.3
    Checking icu_normalizer v2.3.0
    Checking data-encoding v2.11.1
    Checking errno v0.3.14
    Checking socket2 v0.6.5
   Compiling curve25519-dalek v5.0.0
   Compiling parking_lot_core v0.9.12
    Checking percent-encoding v2.3.2
   Compiling either v1.18.0
    Checking form_urlencoded v1.2.2
    Checking signal-hook-registry v1.4.8
   Compiling itertools v0.14.0
    Checking futures-executor v0.3.34
   Compiling tokio-macros v2.7.2
    Checking mio v1.2.2
    Checking idna_adapter v1.2.2
    Checking idna v1.1.0
    Checking unsigned-varint v0.8.0
    Checking futures v0.3.34
    Checking tokio v1.53.1
    Checking url v2.5.8
   Compiling pin-project-internal v1.1.13
   Compiling prost-derive v0.14.4
    Checking signature v3.0.0
    Checking static_assertions v1.1.0
    Checking pin-project v1.1.13
    Checking ed25519 v3.0.0
    Checking parking_lot v0.12.5
   Compiling data-encoding-macro-internal v0.1.19
    Checking hmac v0.13.0
    Checking sha2 v0.11.0
   Compiling match-lookup v0.1.3
    Checking const-str v0.4.3
    Checking prost v0.14.4
    Checking base256emoji v1.0.2
    Checking data-encoding-macro v0.1.21
    Checking hkdf v0.13.0
    Checking ed25519-dalek v3.0.0
    Checking multihash v0.19.5
    Checking base45 v3.2.0
    Checking bs58 v0.5.1
   Compiling pkg-config v0.3.34
    Checking base-x v0.2.11
    Checking multibase v0.9.3
    Checking libp2p-identity v0.3.0
    Checking futures-timer v3.0.3
    Checking arrayref v0.3.9
    Checking fnv v1.0.7
    Checking web-time v1.1.0
    Checking multiaddr v0.19.0
    Checking multistream-select v0.14.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking rw-stream-sink v0.5.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
   Compiling autocfg v1.5.1
    Checking foldhash v0.2.0
    Checking libp2p-core v0.44.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
   Compiling cfg_aliases v0.2.2
   Compiling version_check v0.9.5
   Compiling num-traits v0.2.19
   Compiling heck v0.5.0
   Compiling generic-array v0.14.7
   Compiling cmake v0.1.58
   Compiling fs_extra v1.3.0
   Compiling dunce v1.0.5
    Checking hashbrown v0.17.1
    Checking getrandom v0.2.17
   Compiling aws-lc-sys v0.44.0
   Compiling libp2p-swarm-derive v0.36.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking hashlink v0.12.1
    Checking untrusted v0.9.0
   Compiling ring v0.17.14
   Compiling paste v1.0.15
    Checking libp2p-swarm v0.48.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
   Compiling aws-lc-rs v1.18.0
   Compiling num-conv v0.2.2
   Compiling time-core v0.1.9
   Compiling time-macros v0.2.32
    Checking crypto-common v0.1.7
    Checking block-buffer v0.10.4
    Checking ipnet v2.12.1
    Checking powerfmt v0.2.0
    Checking bitflags v2.13.1
    Checking tinyvec_macros v0.1.1
    Checking deranged v0.5.8
    Checking tinyvec v1.12.0
    Checking digest v0.10.7
    Checking rustls-pki-types v1.15.1
    Checking time v0.3.55
    Checking minimal-lexical v0.2.1
    Checking nom v7.1.3
    Checking netlink-packet-core v0.8.2
   Compiling nix v0.30.1
    Checking asynchronous-codec v0.7.0
   Compiling heapless v0.7.17
   Compiling crossbeam-utils v0.8.22
    Checking cpufeatures v0.2.17
   Compiling rustls v0.23.43
    Checking hex v0.4.3
   Compiling thiserror v1.0.69
    Checking rusticata-macros v4.1.0
    Checking netlink-sys v0.8.8
    Checking futures-bounded v0.3.0
    Checking hash32 v0.2.1
    Checking spin v0.9.9
   Compiling thiserror-impl v1.0.69
   Compiling asn1-rs-impl v0.2.0
   Compiling asn1-rs-derive v0.6.0
   Compiling blake3 v1.8.7
   Compiling libm v0.2.16
    Checking asn1-rs v0.7.2
    Checking netlink-proto v0.12.2
    Checking prost-codec v0.4.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking netlink-packet-route v0.28.0
    Checking prefix-trie v0.8.4
    Checking num-integer v0.1.47
    Checking cobs v0.3.0
   Compiling oid-registry v0.8.1
    Checking arrayvec v0.7.8
    Checking lazy_static v1.5.0
    Checking constant_time_eq v0.4.2
   Compiling crossbeam-epoch v0.9.20
    Checking postcard v1.1.3
    Checking num-bigint v0.4.8
    Checking hickory-proto v0.26.1
    Checking rtnetlink v0.20.0
    Checking sha2 v0.10.9
   Compiling quinn-udp v0.5.15
    Checking base64 v0.22.1
   Compiling getrandom v0.3.4
   Compiling crunchy v0.2.4
    Checking siphasher v1.0.3
    Checking pem v3.0.6
    Checking if-watch v3.2.2
    Checking fastbloom v0.17.0
    Checking swarm-protocol v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-protocol)
    Checking der-parser v10.0.0
    Checking crossbeam-channel v0.5.16
    Checking yasna v0.5.2
   Compiling quinn v0.11.11
    Checking uuid v1.26.0
    Checking rand_pcg v0.10.2
   Compiling async-trait v0.1.92
   Compiling curve25519-dalek v4.1.3
   Compiling snow v0.10.0
    Checking rustc-hash v2.1.3
    Checking lru-slab v0.1.2
   Compiling zmij v1.0.23
    Checking equivalent v1.0.2
    Checking tagptr v0.2.0
    Checking hickory-net v0.26.1
    Checking moka v0.12.16
    Checking rcgen v0.13.2
    Checking x509-parser v0.18.1
    Checking blake2 v0.10.6
    Checking rand_core v0.6.4
    Checking cbor4ii v1.2.2
    Checking resolv-conf v0.7.6
    Checking signature v2.2.0
    Checking yasna v0.6.0
    Checking nohash-hasher v0.2.0
   Compiling serde_json v1.0.151
    Checking libp2p-request-response v0.30.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking yamux v0.14.0
    Checking ed25519 v2.2.3
    Checking hickory-resolver v0.26.1
    Checking uint v0.10.1
    Checking x25519-dalek v3.0.0
    Checking itoa v1.0.18
    Checking libp2p-noise v0.47.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking ed25519-dalek v2.2.0
    Checking libp2p-dns v0.45.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-kad v0.49.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-yamux v0.48.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-autonat v0.16.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-tcp v0.45.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-mdns v0.49.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-relay v0.22.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-dcutr v0.15.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-identify v0.48.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-allow-block-list v0.7.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-ping v0.48.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-connection-limits v0.7.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
   Compiling zstd-sys v2.0.16+zstd.1.5.7
    Checking utf8parse v0.2.2
   Compiling rustix v1.1.4
   Compiling zstd-safe v7.2.4
    Checking anstyle-parse v1.0.0
    Checking option-ext v0.2.0
    Checking is_terminal_polyfill v1.70.2
    Checking linux-raw-sys v0.12.1
    Checking anstyle-query v1.1.5
    Checking anstyle v1.0.14
    Checking regex-syntax v0.8.11
    Checking colorchoice v1.0.5
    Checking anstream v1.0.0
    Checking regex-automata v0.4.18
    Checking dirs-sys v0.4.1
    Checking clap_lex v1.1.0
    Checking strsim v0.11.1
    Checking fastrand v2.5.0
    Checking same-file v1.0.6
    Checking matchers v0.2.0
    Checking walkdir v2.5.0
    Checking tempfile v3.27.0
    Checking clap_builder v4.6.6
    Checking directories v5.0.1
    Checking sharded-slab v0.1.7
   Compiling clap_derive v4.6.4
    Checking tracing-log v0.2.0
    Checking thread_local v1.1.10
    Checking nu-ansi-term v0.50.3
    Checking tracing-subscriber v0.3.23
    Checking clap v4.6.6
    Checking swarm-core v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-core)
    Checking swarm-ipc v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-ipc)
    Checking swarm-consensus v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-consensus)
    Checking sha1 v0.10.7
    Checking fs2 v0.4.3
    Checking zstd v0.13.3
    Checking swarm-storage v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-storage)
    Checking rustls-webpki v0.103.15
    Checking quinn-proto v0.11.17
    Checking futures-rustls v0.26.0
    Checking libp2p-tls v0.7.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p-quic v0.14.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking libp2p v0.57.0 (https://github.com/libp2p/rust-libp2p?rev=b4c6d6dcaccbae6c69bc5e579a50478911c6f157#b4c6d6dc)
    Checking swarm-network v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-network)
    Checking swarm-cli v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 14s
```

## implementation/agent4-transport-admission-clippy.log
```text
    Checking swarm-protocol v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-protocol)
    Checking swarm-network v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-network)
    Checking swarm-consensus v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-consensus)
    Checking swarm-storage v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-storage)
    Checking swarm-core v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-core)
    Checking swarm-ipc v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-ipc)
error: this assertion has a constant value
   --> crates/swarm-network/src/admission.rs:166:9
    |
166 |         assert!(MAX_PENDING_INCOMING_CONNECTIONS < MAX_ESTABLISHED_CONNECTIONS);
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = help: consider moving this into a const block: `const { assert!(..) }`
    = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#assertions_on_constants
    = note: `-D clippy::assertions-on-constants` implied by `-D warnings`
    = help: to override `-D warnings` add `#[allow(clippy::assertions_on_constants)]`

error: this assertion has a constant value
   --> crates/swarm-network/src/admission.rs:167:9
    |
167 |         assert!(MAX_ESTABLISHED_INCOMING_CONNECTIONS <= MAX_ESTABLISHED_CONNECTIONS);
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = help: consider moving this into a const block: `const { assert!(..) }`
    = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#assertions_on_constants

error: this assertion has a constant value
   --> crates/swarm-network/src/admission.rs:169:9
    |
169 |         assert!(MAX_DISCOVERY_PENDING_INCOMING_CONNECTIONS < MAX_DISCOVERY_ESTABLISHED_CONNECTIONS);
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = help: consider moving this into a const block: `const { assert!(..) }`
    = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#assertions_on_constants

error: this assertion has a constant value
   --> crates/swarm-network/src/admission.rs:170:9
    |
170 |         assert!(MAX_DISCOVERY_ESTABLISHED_INCOMING_CONNECTIONS <= MAX_DISCOVERY_ESTABLISHED_CONNECTIONS);
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = help: consider moving this into a const block: `const { assert!(..) }`
    = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#assertions_on_constants

error: this assertion has a constant value
   --> crates/swarm-network/src/admission.rs:171:9
    |
171 |         assert!(MAX_ESTABLISHED_CONNECTIONS_PER_PEER >= 2);
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |
    = help: consider moving this into a const block: `const { assert!(..) }`
    = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#assertions_on_constants

    Checking swarm-cli v0.5.0 (/home/runner/work/swarmcraft/swarmcraft/crates/swarm-cli)
error: could not compile `swarm-network` (lib test) due to 5 previous errors
warning: build failed, waiting for other jobs to finish...
```

## implementation/agent4-transport-admission-format.log
```text
```
