import java.io.BufferedWriter;
import java.io.OutputStreamWriter;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HexFormat;

public final class MockFabricServer {
    private static String required(String name) {
        String value = System.getenv(name);
        if (value == null || value.isBlank()) {
            throw new IllegalStateException("missing environment variable: " + name);
        }
        return value;
    }

    private static String hex(String value) {
        return HexFormat.of().formatHex(value.getBytes(StandardCharsets.UTF_8));
    }

    public static void main(String[] args) throws Exception {
        String host = required("SWARMCRAFT_IPC_HOST");
        int port = Integer.parseInt(required("SWARMCRAFT_IPC_PORT"));
        String token = required("SWARMCRAFT_IPC_TOKEN");
        String worldDirectory = required("SWARMCRAFT_WORLD_DIR");
        String fingerprint = required("SWARMCRAFT_COMPAT_FINGERPRINT");

        Files.writeString(
            Path.of(worldDirectory).resolve("swarmcraft-runtime-smoke.txt"),
            "mutated-by-real-host-process\n",
            StandardCharsets.UTF_8
        );

        try (Socket socket = new Socket(host, port);
             BufferedWriter writer = new BufferedWriter(
                 new OutputStreamWriter(socket.getOutputStream(), StandardCharsets.UTF_8)
             )) {
            writer.write("AUTH\t" + token);
            writer.newLine();
            writer.write(
                "WORLD_INFO\t"
                    + hex("26.1.2") + "\t"
                    + hex("0.19.3") + "\t"
                    + hex(worldDirectory) + "\t"
                    + fingerprint
            );
            writer.newLine();
            writer.flush();
            Thread.sleep(500);
        }
    }
}
