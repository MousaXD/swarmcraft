package dev.swarmcraft.fabric;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.lang.reflect.Method;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.HexFormat;
import java.util.concurrent.atomic.AtomicBoolean;

import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.server.MinecraftServer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public final class SwarmCraftMod implements ModInitializer {
    public static final String MOD_ID = "swarmcraft";
    private static final Logger LOGGER = LoggerFactory.getLogger(MOD_ID);
    private static final Duration CONNECT_TIMEOUT = Duration.ofSeconds(10);
    private static volatile Bridge bridge;

    @Override
    public void onInitialize() {
        ServerLifecycleEvents.SERVER_STARTED.register(SwarmCraftMod::serverStarted);
        ServerLifecycleEvents.SERVER_STOPPING.register(server -> closeBridge());
    }

    private static void serverStarted(MinecraftServer server) {
        String host = System.getenv("SWARMCRAFT_IPC_HOST");
        String port = System.getenv("SWARMCRAFT_IPC_PORT");
        String token = System.getenv("SWARMCRAFT_IPC_TOKEN");
        if (host == null || port == null || token == null) {
            LOGGER.info("SwarmCraft IPC environment is not present; running as a normal Fabric server");
            return;
        }

        try {
            InetAddress address = InetAddress.getByName(host);
            if (!address.isLoopbackAddress()) {
                throw new IllegalArgumentException("SWARMCRAFT_IPC_HOST must resolve to loopback");
            }
            Bridge next = new Bridge(server, address, Integer.parseInt(port), token);
            next.start();
            bridge = next;
        } catch (Exception error) {
            LOGGER.error("Unable to start SwarmCraft lifecycle bridge", error);
        }
    }

    private static void closeBridge() {
        Bridge current = bridge;
        bridge = null;
        if (current != null) {
            current.close();
        }
    }

    private static final class Bridge implements AutoCloseable {
        private final MinecraftServer server;
        private final InetAddress host;
        private final int port;
        private final String token;
        private final AtomicBoolean closed = new AtomicBoolean();
        private Socket socket;
        private BufferedWriter writer;

        private Bridge(MinecraftServer server, InetAddress host, int port, String token) {
            this.server = server;
            this.host = host;
            this.port = port;
            this.token = token;
        }

        private void start() throws IOException {
            socket = new Socket();
            socket.connect(new InetSocketAddress(host, port), Math.toIntExact(CONNECT_TIMEOUT.toMillis()));
            socket.setTcpNoDelay(true);
            writer = new BufferedWriter(new OutputStreamWriter(socket.getOutputStream(), StandardCharsets.UTF_8));
            send("AUTH\t" + token);
            sendWorldInfo();

            Thread reader = new Thread(this::readerLoop, "swarmcraft-ipc-reader");
            reader.setDaemon(true);
            reader.start();
            LOGGER.info("SwarmCraft lifecycle bridge connected to local daemon");
        }

        private void sendWorldInfo() throws IOException {
            String minecraft = FabricLoader.getInstance()
                .getModContainer("minecraft")
                .map(container -> container.getMetadata().getVersion().getFriendlyString())
                .orElse("unknown");
            String loader = FabricLoader.getInstance()
                .getModContainer("fabricloader")
                .map(container -> container.getMetadata().getVersion().getFriendlyString())
                .orElse("unknown");
            String worldDirectory = valueOrEmpty(System.getenv("SWARMCRAFT_WORLD_DIR"));
            String compatibility = valueOrEmpty(System.getenv("SWARMCRAFT_COMPAT_FINGERPRINT"));
            send("WORLD_INFO\t" + encode(minecraft) + "\t" + encode(loader) + "\t" + encode(worldDirectory) + "\t" + compatibility);
        }

        private void readerLoop() {
            try (BufferedReader reader = new BufferedReader(new InputStreamReader(socket.getInputStream(), StandardCharsets.UTF_8))) {
                String line;
                while (!closed.get() && (line = reader.readLine()) != null) {
                    handle(line);
                }
            } catch (IOException error) {
                if (!closed.get()) {
                    LOGGER.error("SwarmCraft IPC connection failed", error);
                }
            }
        }

        private void handle(String line) {
            String[] fields = line.split("\\t", -1);
            if (fields.length != 2) {
                LOGGER.warn("Ignoring malformed SwarmCraft IPC command");
                return;
            }
            String requestId = fields[1];
            switch (fields[0]) {
                case "SAVE_BARRIER" -> server.execute(() -> runSaveBarrier(requestId, false));
                case "PREPARE_SHUTDOWN" -> server.execute(() -> runSaveBarrier(requestId, true));
                default -> LOGGER.warn("Ignoring unknown SwarmCraft IPC command: {}", fields[0]);
            }
        }

        private void runSaveBarrier(String requestId, boolean shutdown) {
            try {
                saveEverything(server);
                send((shutdown ? "READY_FOR_SHUTDOWN" : "SAVE_COMPLETE") + "\t" + requestId);
                if (shutdown) {
                    requestServerStop(server);
                }
            } catch (Exception error) {
                LOGGER.error("SwarmCraft save barrier failed", error);
                try {
                    send("ERROR\t" + requestId + "\tSAVE_FAILED");
                } catch (IOException ignored) {
                    LOGGER.error("Unable to report save-barrier failure to SwarmCraft daemon");
                }
            }
        }

        private synchronized void send(String line) throws IOException {
            if (closed.get()) {
                throw new IOException("SwarmCraft bridge is closed");
            }
            writer.write(line);
            writer.newLine();
            writer.flush();
        }

        @Override
        public void close() {
            if (!closed.compareAndSet(false, true)) {
                return;
            }
            try {
                if (socket != null) {
                    socket.close();
                }
            } catch (IOException error) {
                LOGGER.debug("Error while closing SwarmCraft IPC socket", error);
            }
        }
    }

    private static void saveEverything(MinecraftServer server) throws Exception {
        invokeNoArgIfPresent(server, "savePlayers");
        Method save = findBooleanSaveMethod(server.getClass());
        Object result = save.invoke(server, false, true, true);
        if (result instanceof Boolean success && !success) {
            throw new IOException("Minecraft reported an unsuccessful save");
        }
    }

    private static void requestServerStop(MinecraftServer server) throws Exception {
        for (String name : new String[] {"stopServer", "stop", "halt"}) {
            try {
                Method method = server.getClass().getMethod(name);
                method.setAccessible(true);
                method.invoke(server);
                return;
            } catch (NoSuchMethodException ignored) {
            }
            try {
                Method method = server.getClass().getMethod(name, boolean.class);
                method.setAccessible(true);
                method.invoke(server, false);
                return;
            } catch (NoSuchMethodException ignored) {
            }
        }
        throw new NoSuchMethodException("MinecraftServer has no supported stop method");
    }

    private static Method findBooleanSaveMethod(Class<?> type) throws NoSuchMethodException {
        for (String name : new String[] {"saveEverything", "saveAll", "saveAllChunks", "save"}) {
            try {
                Method method = type.getMethod(name, boolean.class, boolean.class, boolean.class);
                method.setAccessible(true);
                return method;
            } catch (NoSuchMethodException ignored) {
            }
        }
        throw new NoSuchMethodException("MinecraftServer has no supported three-boolean save method");
    }

    private static void invokeNoArgIfPresent(Object target, String name) throws Exception {
        try {
            Method method = target.getClass().getMethod(name);
            method.setAccessible(true);
            method.invoke(target);
        } catch (NoSuchMethodException ignored) {
        }
    }

    private static String encode(String value) {
        return HexFormat.of().formatHex(value.getBytes(StandardCharsets.UTF_8));
    }

    private static String valueOrEmpty(String value) {
        return value == null ? "" : value;
    }
}
