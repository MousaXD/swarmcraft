# SwarmCraft Fabric integration

Stage 7 target. The Fabric mod stays thin and owns Minecraft lifecycle integration only: save barriers,
world path/version/fingerprint reporting, readiness, launch/stop coordination, and player connection
information. Distributed-systems logic remains in Rust.
