# SwarmCraft Fabric integration

Stage 7 target. The Fabric mod stays thin and owns Minecraft lifecycle integration only: save barriers,
world path/version/fingerprint reporting, readiness, launch/stop coordination, and player connection
information. Distributed-systems logic remains in Rust.

The isolated #65 ownerless permission guard and explicit test-only probe are
described in [the permission RFC](../../docs/rfcs/ownerless-permissions.md).
They do not authorize live ownerless admission or V1 authority.

The optional [Fabric client reconnect component](../../docs/CLIENT_RECONNECT.md)
has a separate client-only JAR and explicit `clientReconnectJar` /
`clientReconnectCheck` tasks. It is not included in the server bridge,
managed runtime installation or release artifacts.
