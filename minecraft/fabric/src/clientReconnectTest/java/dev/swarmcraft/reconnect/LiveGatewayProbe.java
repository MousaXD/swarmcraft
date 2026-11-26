package dev.swarmcraft.reconnect;

import com.google.gson.Gson;
import com.google.gson.JsonParser;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.Map;

/**
 * Explicit test-only process entry point used by the disposable real-daemon
 * gateway handoff gate. Runs the production client's actual native subprocess
 * and proof decoder, then opens one fresh Minecraft status connection.
 * This is not a Fabric game login, UI test or evidence of player identity.
 */
public final class LiveGatewayProbe {
    public static void main(String[] args) throws Exception {
        if (args.length != 4) {
            throw new IllegalArgumentException("expected config, mode, authority, epoch");
        }
        var config = ReconnectPolicy.Config.load(Path.of(args[0]));
        if (!args[1].equals("allow") && !args[1].equals("deny")) {
            throw new IllegalArgumentException("invalid live probe mode");
        }
        var window = new ReconnectPolicy.AttemptWindow(System.nanoTime());
        if (!window.begin(System.nanoTime())) throw new AssertionError("probe not started");
        var result = RouteProbe.check(config, ReconnectPolicy.PROBE_NS);
        if (args[1].equals("deny")) {
            if (result.proof().isPresent()) throw new AssertionError("unexpected eligible route");
            window.stop();
            System.out.println("{\"outcome\":\"refused\"}");
            return;
        }
        var proof = result.proof().orElseThrow(() -> new AssertionError("native route was not usable"));
        if (!proof.authority().equals(args[2]) || proof.epoch() != Long.parseLong(args[3])) {
            throw new AssertionError("native route differs from expected authority/epoch");
        }
        if (!proof.usable(config, result.requestedAt(), System.currentTimeMillis())
                || !window.authorizeConnection(System.nanoTime())) {
            throw new AssertionError("native route expired before fresh connection");
        }
        String version = minecraftStatus(config);
        System.out.println(new Gson().toJson(Map.of(
                "outcome", "minecraft_status",
                "world", proof.world(), "gateway", proof.gateway(),
                "authority", proof.authority(), "epoch", proof.epoch(), "fence", proof.fence(),
                "snapshot", proof.snapshot(), "version", version)));
    }

    static String minecraftStatus(ReconnectPolicy.Config config) throws IOException {
        // Literal address bytes: no host resolver or SRV lookup.
        var address = new InetSocketAddress(InetAddress.getByAddress(new byte[]{127, 0, 0, 1}), config.port());
        try (var socket = new Socket()) {
            socket.connect(address, 5000);
            socket.setSoTimeout(25_000);
            var handshake = new ByteArrayOutputStream();
            handshake.write(0);
            putVarint(-1, handshake);
            byte[] host = "127.0.0.1".getBytes(StandardCharsets.UTF_8);
            putVarint(host.length, handshake);
            handshake.writeBytes(host);
            handshake.write((config.port() >>> 8) & 255);
            handshake.write(config.port() & 255);
            handshake.write(1);
            var framed = new ByteArrayOutputStream();
            putVarint(handshake.size(), framed);
            framed.writeBytes(handshake.toByteArray());
            framed.writeBytes(new byte[]{1, 0});
            socket.getOutputStream().write(framed.toByteArray());
            InputStream input = socket.getInputStream();
            int length = varint(input);
            if (length < 1 || length > 1_048_576) throw new IOException("invalid Minecraft response length");
            byte[] body = input.readNBytes(length);
            if (body.length != length) throw new IOException("truncated Minecraft response");
            var packet = new ByteArrayInputStream(body);
            if (varint(packet) != 0) throw new IOException("unexpected Minecraft packet");
            int jsonLength = varint(packet);
            if (jsonLength != packet.available()) throw new IOException("invalid Minecraft status JSON length");
            var json = JsonParser.parseString(new String(packet.readNBytes(jsonLength), StandardCharsets.UTF_8));
            String version = json.getAsJsonObject().getAsJsonObject("version").get("name").getAsString();
            if (!version.contains("26.1.2")) throw new IOException("wrong real Minecraft server version");
            return version;
        }
    }

    private static void putVarint(int value, ByteArrayOutputStream output) {
        do {
            int next = value & 127;
            value >>>= 7;
            output.write(next | (value == 0 ? 0 : 128));
        } while (value != 0);
    }

    private static int varint(InputStream input) throws IOException {
        int value = 0;
        for (int shift = 0; shift < 35; shift += 7) {
            int next = input.read();
            if (next < 0 || (shift == 28 && (next & 240) != 0)) throw new IOException("invalid Minecraft VarInt");
            value |= (next & 127) << shift;
            if ((next & 128) == 0) return value;
        }
        throw new IOException("oversized Minecraft VarInt");
    }
}
