package dev.swarmcraft.reconnect;

import com.google.gson.Gson;

import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.FutureTask;

/** Dependency-free policy and real subprocess regression runner. */
public final class ReconnectPolicyTest {
    private static final String WORLD = "scworld:" + "a".repeat(64);
    private static final String GATEWAY = "127.0.0.1:25570";
    private static final long NOW = 2_000_000L;
    private static final Gson JSON = new Gson();
    private static int assertions;

    public static void main(String[] args) throws Exception {
        Path root = Files.createTempDirectory("swarmcraft-reconnect-test-");
        try {
            var config = configChecks(root);
            proofChecks(config);
            windowChecks();
            subprocessChecks(root);
            actualUnavailableDaemon(root);
            System.out.println("SwarmCraft reconnect: " + assertions + " assertions passed");
        } finally {
            try (var files = Files.walk(root)) {
                for (Path path : files.sorted(java.util.Comparator.reverseOrder()).toList()) {
                    Files.deleteIfExists(path);
                }
            }
        }
    }

    private static void check(boolean value, String reason) {
        assertions++;
        if (!value) {
            throw new AssertionError(reason);
        }
    }

    private static void reject(Throwing call, String reason) throws Exception {
        try {
            call.run();
        } catch (IOException | RuntimeException expected) {
            assertions++;
            return;
        }
        throw new AssertionError(reason);
    }

    @FunctionalInterface
    private interface Throwing { void run() throws Exception; }

    private static Path javaExecutable() {
        return Path.of(System.getProperty("java.home"), "bin",
                System.getProperty("os.name").startsWith("Windows") ? "java.exe" : "java");
    }

    private static ReconnectPolicy.Config configChecks(Path root) throws Exception {
        Path path = root.resolve("config.json");
        Map<String, Object> fields = new LinkedHashMap<>();
        fields.put("cli", javaExecutable().toString());
        fields.put("data_dir", root.toString());
        fields.put("world_id", WORLD);
        fields.put("local_gateway", GATEWAY);
        Files.writeString(path, JSON.toJson(fields));
        var config = ReconnectPolicy.Config.load(path);
        check(config.port() == 25570 && config.gateway().equals(GATEWAY), "exact loopback config");
        check(!config.autoReconnect(), "automatic reconnect defaults to off");
        var automatic = new LinkedHashMap<>(fields);
        automatic.put("auto_reconnect", true);
        Files.writeString(path, JSON.toJson(automatic));
        check(ReconnectPolicy.Config.load(path).autoReconnect(), "explicit automatic opt-in");
        automatic.put("auto_reconnect", "true");
        Files.writeString(path, JSON.toJson(automatic));
        reject(() -> ReconnectPolicy.Config.load(path), "no implicit automatic opt-in");
        var loopback = new InetSocketAddress(InetAddress.getByAddress(new byte[]{127,0,0,1}), 25570);
        check(config.matchesJoinedGateway(GATEWAY, loopback, false), "exact joined socket");
        check(!config.matchesJoinedGateway("localhost:25570", loopback, false), "alias not opted in");
        check(!config.matchesJoinedGateway(GATEWAY, loopback, true), "memory connection not eligible");
        check(!config.matchesJoinedGateway(GATEWAY, InetSocketAddress.createUnresolved("127.0.0.1", 25570), false),
                "unresolved socket not eligible");
        check(!config.matchesJoinedGateway(GATEWAY, new InetSocketAddress(loopback.getAddress(), 25571), false),
                "wrong connected port");
        check(!config.matchesJoinedGateway(GATEWAY,
                new InetSocketAddress(InetAddress.getByAddress(new byte[]{127,0,0,2}), 25570), false),
                "wrong connected address");
        check(config.command().equals(List.of(javaExecutable().toRealPath().toString(), "--data-dir",
                root.toRealPath().toString(), "world", "game-route-status", WORLD, "--json")),
                "fixed argv without shell or selection");
        for (String address : List.of("localhost:25570", "127.0.0.2:25570", "127.0.0.1:0",
                "127.0.0.1:025570", "127.0.0.1:65536", "127.0.0.1:25565", "[::1]:25570", "example.org:25565",
                "127.0.0.1:25570\n")) {
            var altered = new LinkedHashMap<>(fields);
            altered.put("local_gateway", address);
            Files.writeString(path, JSON.toJson(altered));
            reject(() -> ReconnectPolicy.Config.load(path), "unsafe configured address");
        }
        for (String key : fields.keySet()) {
            var altered = new LinkedHashMap<>(fields);
            altered.remove(key);
            Files.writeString(path, JSON.toJson(altered));
            reject(() -> ReconnectPolicy.Config.load(path), "missing configuration");
        }
        var altered = new LinkedHashMap<>(fields);
        altered.put("cli", "swarmcraft");
        Files.writeString(path, JSON.toJson(altered));
        reject(() -> ReconnectPolicy.Config.load(path), "PATH lookup forbidden");
        altered = new LinkedHashMap<>(fields);
        altered.put("world_id", "scworld:" + "x".repeat(64));
        Files.writeString(path, JSON.toJson(altered));
        reject(() -> ReconnectPolicy.Config.load(path), "malformed world");
        Files.writeString(path, JSON.toJson(fields).replaceFirst("\\{", "{\"world_id\":\"" + WORLD + "\","));
        reject(() -> ReconnectPolicy.Config.load(path), "duplicate configured world");
        Files.writeString(path, " ".repeat(ReconnectPolicy.MAX_DOCUMENT_BYTES + 1));
        reject(() -> ReconnectPolicy.Config.load(path), "oversized config");
        return config;
    }

    private static Map<String, Object> proof() {
        Map<String, Object> fields = new LinkedHashMap<>();
        fields.put("world_id", WORLD);
        fields.put("state", "verified");
        fields.put("quorum_verified", true);
        fields.put("hosting_consent_verified", true);
        fields.put("server_ready", true);
        fields.put("route_verified", true);
        fields.put("local_gateway", GATEWAY);
        fields.put("authority_peer_id", "scpeer:" + "b".repeat(64));
        fields.put("epoch", 2L);
        fields.put("fencing_token", 3L);
        fields.put("snapshot_hash", "c".repeat(64));
        fields.put("verified_at_unix_ms", NOW);
        fields.put("valid_until_unix_ms", NOW + 3_000);
        fields.put("recent_session", Map.of("state", "session_ended", "age_ms", 1));
        return fields;
    }

    private static byte[] bytes(Object value) {
        return JSON.toJson(value).getBytes(StandardCharsets.UTF_8);
    }

    private static boolean accepted(byte[] bytes, ReconnectPolicy.Config config, long requested, long now) {
        return ReconnectPolicy.decode(bytes, config, requested, now).isPresent();
    }

    private static void proofChecks(ReconnectPolicy.Config config) {
        Map<String, Object> good = proof();
        check(accepted(bytes(good), config, NOW - 500, NOW + 100), "fresh exact proof");
        Map<String, Object> invalid = new LinkedHashMap<>();
        invalid.put("world_id", "scworld:" + "d".repeat(64));
        invalid.put("state", "unavailable");
        invalid.put("quorum_verified", false);
        invalid.put("hosting_consent_verified", false);
        invalid.put("server_ready", false);
        invalid.put("route_verified", false);
        invalid.put("local_gateway", "127.0.0.1:25571");
        invalid.put("authority_peer_id", "bad");
        invalid.put("epoch", 0);
        invalid.put("fencing_token", 0);
        invalid.put("snapshot_hash", "bad");
        invalid.put("verified_at_unix_ms", NOW - 6_000);
        invalid.put("valid_until_unix_ms", NOW + 16_000);
        invalid.put("diagnostic", "gateway_unavailable");
        for (var field : invalid.entrySet()) {
            var changed = new LinkedHashMap<>(good);
            changed.put(field.getKey(), field.getValue());
            check(!accepted(bytes(changed), config, NOW - 500, NOW + 100), "invalid " + field.getKey());
        }
        for (String key : good.keySet()) {
            if (key.equals("recent_session")) continue;
            var changed = new LinkedHashMap<>(good);
            changed.remove(key);
            check(!accepted(bytes(changed), config, NOW - 500, NOW + 100), "missing " + key);
        }
        var changed = new LinkedHashMap<>(good);
        changed.put("quorum_verified", "true");
        check(!accepted(bytes(changed), config, NOW - 500, NOW + 100), "no bool coercion");
        changed = new LinkedHashMap<>(good);
        changed.put("epoch", "2");
        check(!accepted(bytes(changed), config, NOW - 500, NOW + 100), "no numeric coercion");
        changed = new LinkedHashMap<>(good);
        changed.put("unexpected", true);
        check(!accepted(bytes(changed), config, NOW - 500, NOW + 100), "unknown fields fail closed");
        String text = JSON.toJson(good);
        for (String malformed : List.of(text + "{}", text.replaceFirst("\\{", "{\"state\":\"verified\","),
                "{broken", " ".repeat(ReconnectPolicy.MAX_DOCUMENT_BYTES + 1))) {
            check(!accepted(malformed.getBytes(StandardCharsets.UTF_8), config, NOW - 500, NOW + 100),
                    "malformed or duplicate proof");
        }
        check(!accepted(bytes(good), config, NOW + 2_100, NOW + 2_100), "pre-request proof");
        check(!accepted(bytes(good), config, NOW - 500, NOW + 2_900), "insufficient remaining lifetime");
        check(!accepted(bytes(good), config, NOW - 500, NOW + 3_001), "expired proof");
        check(!accepted(bytes(good), config, NOW - 500, NOW - 2_001), "clock rollback");
        var usable = ReconnectPolicy.decode(bytes(good), config, NOW - 500, NOW + 100).orElseThrow();
        check(!usable.usable(config, NOW - 500, NOW + 3_100), "recheck at dispatch");
    }

    private static void windowChecks() {
        long start = 1000;
        var w = new ReconnectPolicy.AttemptWindow(start);
        check(w.begin(start), "first attempt");
        check(!w.begin(start), "one in-flight attempt");
        w.failed(start + 1);
        check(!w.begin(start + ReconnectPolicy.RETRY_NS), "retry delay");
        check(w.begin(start + ReconnectPolicy.RETRY_NS + 1), "second attempt");
        w.failed(start + ReconnectPolicy.RETRY_NS + 1);
        check(w.begin(start + 2 * ReconnectPolicy.RETRY_NS + 1), "third attempt");
        w.failed(start + 2 * ReconnectPolicy.RETRY_NS + 1);
        check(w.exhausted(start + 3 * ReconnectPolicy.RETRY_NS), "bounded retry count");
        check(!w.begin(start + 3 * ReconnectPolicy.RETRY_NS), "no fourth attempt");
        check(!w.authorizeConnection(start + 3 * ReconnectPolicy.RETRY_NS), "no proof without in-flight attempt");
        w = new ReconnectPolicy.AttemptWindow(start);
        check(w.begin(start) && w.authorizeConnection(start + 1), "one connection");
        check(!w.authorizeConnection(start + 2) && !w.begin(start + 2), "connection consumes request");
        w = new ReconnectPolicy.AttemptWindow(start);
        w.begin(start);
        w.stop();
        check(!w.authorizeConnection(start + 1), "cancel invalidates late proof");
        w = new ReconnectPolicy.AttemptWindow(start);
        w.begin(start);
        check(!w.authorizeConnection(start + ReconnectPolicy.WINDOW_NS), "deadline invalidates late proof");
    }

    private static List<String> fake(String mode, Path pid) throws Exception {
        Path cp = Path.of(ReconnectPolicyTest.class.getProtectionDomain().getCodeSource().getLocation().toURI());
        List<String> command = new ArrayList<>(List.of(javaExecutable().toString(), "-cp", cp.toString(),
                FakeCli.class.getName(), mode));
        if (pid != null) command.add(pid.toString());
        return command;
    }

    private static void subprocessChecks(Path root) throws Exception {
        long budget = Duration.ofSeconds(5).toNanos();
        check(new String(RouteProbe.run(fake("ok", null), budget), StandardCharsets.UTF_8).equals("exact\n"),
                "actual subprocess output");
        reject(() -> RouteProbe.run(fake("nonzero", null), budget), "nonzero exit refused");
        reject(() -> RouteProbe.run(fake("flood", null), budget), "bounded subprocess output");
        reject(() -> RouteProbe.run(fake("sleep", null), Duration.ofMillis(400).toNanos()), "subprocess timeout");
        Path marker = root.resolve("probe-pid");
        var command = fake("sleep", marker);
        var task = new FutureTask<>(() -> RouteProbe.run(command, budget));
        Thread thread = Thread.ofPlatform().start(task);
        try {
            long until = System.nanoTime() + Duration.ofSeconds(3).toNanos();
            while (!Files.exists(marker) && System.nanoTime() < until) Thread.sleep(10);
            check(Files.exists(marker), "actual cancellable subprocess started");
            long pid = Long.parseLong(Files.readString(marker));
            check(task.cancel(true), "in-flight cancellation");
            thread.join(2000);
            check(!thread.isAlive(), "cancelled probe thread terminated");
            until = System.nanoTime() + Duration.ofSeconds(2).toNanos();
            while (ProcessHandle.of(pid).map(ProcessHandle::isAlive).orElse(false) && System.nanoTime() < until) Thread.sleep(10);
            check(ProcessHandle.of(pid).map(p -> !p.isAlive()).orElse(true), "cancelled subprocess terminated");
        } finally {
            task.cancel(true);
            thread.join(2000);
        }
    }

    private static void actualUnavailableDaemon(Path root) throws Exception {
        String executable = System.getenv("SWARMCRAFT_RECONNECT_TEST_CLI");
        if (executable == null || executable.isBlank()) {
            System.out.println("Optional native no-daemon check not requested");
            return;
        }
        Path cli = Path.of(executable).toRealPath();
        Path profile = Files.createDirectory(root.resolve("isolated-native-profile"));
        var config = new ReconnectPolicy.Config(cli, profile, WORLD, GATEWAY, 25570, false);
        // No production daemon or user profile is involved. An unavailable
        // live endpoint, including a nonzero exit, must not authorize a route.
        check(RouteProbe.check(config, Duration.ofSeconds(5).toNanos()).proof().isEmpty(),
                "actual SwarmCraft CLI with no live daemon refused");
    }

    public static final class FakeCli {
        public static void main(String[] args) throws Exception {
            switch (args[0]) {
                case "ok" -> System.out.print("exact\n");
                case "nonzero" -> { System.out.print("exact\n"); System.exit(3); }
                case "flood" -> System.out.print("x".repeat(8192));
                case "sleep" -> {
                    if (args.length > 1) {
                        Path marker = Path.of(args[1]);
                        Path temporary = marker.resolveSibling(marker.getFileName() + ".tmp");
                        Files.writeString(temporary, Long.toString(ProcessHandle.current().pid()));
                        Files.move(temporary, marker, java.nio.file.StandardCopyOption.ATOMIC_MOVE);
                    }
                    Thread.sleep(60_000);
                }
                default -> System.exit(2);
            }
        }
    }
}
