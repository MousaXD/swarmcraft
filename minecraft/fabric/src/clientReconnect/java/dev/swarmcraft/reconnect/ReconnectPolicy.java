package dev.swarmcraft.reconnect;

import com.google.gson.Strictness;
import com.google.gson.stream.JsonReader;
import com.google.gson.stream.JsonToken;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStreamReader;
import java.net.InetSocketAddress;
import java.net.SocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.time.Duration;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;

/** Local UI policy only. The Rust gateway remains the sole route admission boundary. */
final class ReconnectPolicy {
    static final int MAX_DOCUMENT_BYTES = 4096;
    static final int MAX_ATTEMPTS = 3;
    static final long WINDOW_NS = Duration.ofSeconds(45).toNanos();
    static final long RETRY_NS = Duration.ofSeconds(2).toNanos();
    static final long PROBE_NS = Duration.ofSeconds(22).toNanos();
    private static final long MAX_AGE_MS = 5_000;
    private static final long MAX_LIFETIME_MS = 15_000;
    private static final long SKEW_MS = 2_000;
    private static final long MIN_REMAINING_MS = 250;

    private static final Set<String> CONFIG_FIELDS = Set.of("cli", "data_dir", "world_id", "local_gateway");
    private static final Set<String> PROOF_FIELDS = Set.of(
            "world_id", "state", "quorum_verified", "hosting_consent_verified", "server_ready",
            "route_verified", "local_gateway", "authority_peer_id", "epoch", "fencing_token",
            "snapshot_hash", "verified_at_unix_ms", "valid_until_unix_ms");
    private static final Set<String> OPTIONAL_FIELDS = Set.of("diagnostic", "recent_session");

    record Config(Path cli, Path dataDir, String world, String gateway, int port, boolean autoReconnect) {
        static Config load(Path path) throws IOException {
            if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
                throw new IOException("Reconnect configuration must be a regular local file");
            }
            byte[] bytes;
            try (var in = Files.newInputStream(path, LinkOption.NOFOLLOW_LINKS)) {
                bytes = in.readNBytes(MAX_DOCUMENT_BYTES + 1);
            }
            Map<String, Object> values = readObject(bytes, CONFIG_FIELDS, Set.of("auto_reconnect"));
            if (!values.keySet().containsAll(CONFIG_FIELDS)
                    || (values.containsKey("auto_reconnect") && !(values.get("auto_reconnect") instanceof Boolean))) {
                throw new IOException("Incomplete reconnect configuration");
            }
            String world = string(values, "world_id");
            String gateway = string(values, "local_gateway");
            if (!world.matches("scworld:[0-9a-f]{64}") || !gateway.matches("127\\.0\\.0\\.1:[1-9][0-9]{0,4}")) {
                throw new IOException("Reconnect requires a pinned world and exact IPv4 loopback entry");
            }
            int port = Integer.parseInt(gateway.substring("127.0.0.1:".length()));
            // Vanilla's default Minecraft port invokes DNS SRV lookup,
            // even for a numeric host. The managed gateway uses a
            // separate reserved port; never enter that redirect path.
            if (port > 65535 || port == 25565) {
                throw new IOException("Invalid loopback port");
            }
            Path cli = absolute(string(values, "cli"));
            Path dataDir = absolute(string(values, "data_dir"));
            if (!Files.isRegularFile(cli) || !Files.isExecutable(cli) || !Files.isDirectory(dataDir)) {
                throw new IOException("Select an existing trusted SwarmCraft executable and profile");
            }
            return new Config(cli, dataDir, world, gateway, port, Boolean.TRUE.equals(values.get("auto_reconnect")));
        }

        private static Path absolute(String value) throws IOException {
            Path path = Path.of(value);
            if (!path.isAbsolute()) {
                throw new IOException("Reconnect paths must be absolute");
            }
            return path.toRealPath();
        }

        List<String> command() {
            // Fixed argument structure; neither Minecraft nor route output
            // can change the executable, profile, operation, or world.
            return List.of(cli.toString(), "--data-dir", dataDir.toString(), "world",
                    "game-route-status", world, "--json");
        }

        boolean matchesJoinedGateway(String enteredAddress, SocketAddress connectedAddress, boolean memoryConnection) {
            return !memoryConnection && gateway.equals(enteredAddress)
                    && connectedAddress instanceof InetSocketAddress socket && socket.getAddress() != null
                    && socket.getAddress().getHostAddress().equals("127.0.0.1") && socket.getPort() == port;
        }
    }

    record Proof(String world, String gateway, long epoch, long fence, String authority,
                 String snapshot, long verifiedAt, long validUntil) {
        boolean usable(Config config, long requestedAt, long now) {
            return world.equals(config.world()) && gateway.equals(config.gateway())
                    && config.port() > 0 && config.port() <= 65535 && config.port() != 25565
                    && epoch > 0 && fence > 0 && authority.matches("scpeer:[0-9a-f]{64}")
                    && snapshot.matches("[0-9a-f]{64}") && requestedAt > 0 && now >= requestedAt
                    && verifiedAt > 0 && verifiedAt >= requestedAt - SKEW_MS
                    && verifiedAt <= now + SKEW_MS && now - verifiedAt <= MAX_AGE_MS
                    && validUntil > verifiedAt && validUntil - verifiedAt <= MAX_LIFETIME_MS
                    && validUntil - now >= MIN_REMAINING_MS;
        }
    }

    static Optional<Proof> decode(byte[] bytes, Config config, long requestedAt, long now) {
        try {
            var values = readObject(bytes, PROOF_FIELDS, OPTIONAL_FIELDS);
            if (!values.keySet().containsAll(PROOF_FIELDS) || !"verified".equals(values.get("state"))
                    || values.get("diagnostic") != null
                    || !Boolean.TRUE.equals(values.get("quorum_verified"))
                    || !Boolean.TRUE.equals(values.get("hosting_consent_verified"))
                    || !Boolean.TRUE.equals(values.get("server_ready"))
                    || !Boolean.TRUE.equals(values.get("route_verified"))) {
                return Optional.empty();
            }
            Proof proof = new Proof(string(values, "world_id"), string(values, "local_gateway"),
                    positiveInteger(values, "epoch"), positiveInteger(values, "fencing_token"),
                    string(values, "authority_peer_id"), string(values, "snapshot_hash"),
                    positiveInteger(values, "verified_at_unix_ms"), positiveInteger(values, "valid_until_unix_ms"));
            return proof.usable(config, requestedAt, now) ? Optional.of(proof) : Optional.empty();
        } catch (IOException | RuntimeException invalid) {
            return Optional.empty();
        }
    }

    private static String string(Map<String, Object> values, String key) throws IOException {
        if (!(values.get(key) instanceof String value) || value.isEmpty()) {
            throw new IOException("Invalid reconnect string");
        }
        return value;
    }

    private static long positiveInteger(Map<String, Object> values, String key) throws IOException {
        if (!(values.get(key) instanceof Long value) || value <= 0) {
            throw new IOException("Invalid reconnect integer");
        }
        return value;
    }

    /** Strict, bounded object reader. Reject duplicate keys and type coercion. */
    private static Map<String, Object> readObject(byte[] bytes, Set<String> required, Set<String> optional)
            throws IOException {
        if (bytes.length == 0 || bytes.length > MAX_DOCUMENT_BYTES) {
            throw new IOException("Reconnect document size outside bounds");
        }
        var decoder = StandardCharsets.UTF_8.newDecoder();
        try (var reader = new JsonReader(new InputStreamReader(new ByteArrayInputStream(bytes), decoder))) {
            reader.setStrictness(Strictness.STRICT);
            reader.beginObject();
            Map<String, Object> values = new HashMap<>();
            while (reader.hasNext()) {
                String name = reader.nextName();
                if ((!required.contains(name) && !optional.contains(name)) || values.containsKey(name)) {
                    throw new IOException("Unexpected or duplicate reconnect field");
                }
                Object value = switch (reader.peek()) {
                    case STRING -> reader.nextString();
                    case BOOLEAN -> reader.nextBoolean();
                    case NUMBER -> {
                        String raw = reader.nextString();
                        if (!raw.matches("[0-9]{1,19}")) {
                            throw new IOException("Invalid reconnect number");
                        }
                        yield Long.parseLong(raw);
                    }
                    case NULL -> {
                        reader.nextNull();
                        yield null;
                    }
                    case BEGIN_OBJECT -> {
                        if (!name.equals("recent_session")) {
                            throw new IOException("Unexpected nested reconnect field");
                        }
                        reader.beginObject();
                        var seen = new java.util.HashSet<String>();
                        while (reader.hasNext()) {
                            String child = reader.nextName();
                            if (!seen.add(child)) {
                                throw new IOException("Duplicate session diagnostic field");
                            }
                            if (child.equals("state") && reader.peek() == JsonToken.STRING) {
                                reader.nextString();
                            } else if (child.equals("age_ms") && reader.peek() == JsonToken.NUMBER) {
                                String raw = reader.nextString();
                                if (!raw.matches("[0-9]{1,19}")) {
                                    throw new IOException("Invalid session age");
                                }
                            } else {
                                throw new IOException("Unexpected session diagnostic");
                            }
                        }
                        reader.endObject();
                        if (!seen.equals(Set.of("state", "age_ms"))) {
                            throw new IOException("Incomplete session diagnostic");
                        }
                        yield Boolean.TRUE; // purely informational; never authorizes a route
                    }
                    default -> throw new IOException("Unexpected reconnect field type");
                };
                values.put(name, value);
            }
            reader.endObject();
            if (reader.peek() != JsonToken.END_DOCUMENT) {
                throw new IOException("Trailing reconnect document");
            }
            return values;
        }
    }

    /** All elapsed-time decisions are monotonic and local to one opt-in. */
    static final class AttemptWindow {
        private final long started;
        private long retryAt;
        private int attempts;
        private boolean inFlight;
        private boolean stopped;

        AttemptWindow(long now) {
            started = now;
            retryAt = now;
        }

        int attempts() { return attempts; }
        boolean expired(long now) { return now - started >= WINDOW_NS; }
        boolean exhausted(long now) { return stopped || expired(now) || (!inFlight && attempts >= MAX_ATTEMPTS); }
        long remaining(long now) { return Math.max(0, WINDOW_NS - (now - started)); }
        boolean begin(long now) {
            if (inFlight || exhausted(now) || now - retryAt < 0) {
                return false;
            }
            attempts++;
            inFlight = true;
            return true;
        }
        void failed(long now) {
            inFlight = false;
            retryAt = now + RETRY_NS;
        }
        boolean authorizeConnection(long now) {
            if (stopped || !inFlight || expired(now)) {
                return false;
            }
            stop();
            return true;
        }
        void stop() {
            stopped = true;
            inFlight = false;
        }
    }

    private ReconnectPolicy() {}
}
