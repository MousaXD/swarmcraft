package dev.swarmcraft.fabric;

import com.google.gson.JsonParser;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import net.minecraft.server.dedicated.DedicatedServerProperties;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionLevel;

/**
 * Non-activatable #65 permission lab. The trusted local launcher obtains this
 * immutable pin from the strict Rust certificate verifier. It is NOT a signed
 * authority permit, a remote proof, a real player-admission policy, or OS
 * attestation. No production managed launch consumes it.
 */
public final class OwnerlessPermissionGuard {
    public static final String ENV = "SWARMCRAFT_OWNERLESS_LAB_BINDING";
    // Golden hash of the Rust OWNERLESS_NO_OP_V1 invariant set.
    public static final String POLICY_HASH = "c694ae9067292c7ea3da7cc5a825e8ff0b370cccfa6267e45dc3a6c96f76ef4f";
    private static final String BINDING = validate(System.getenv(ENV));

    private OwnerlessPermissionGuard() {}

    private static String validate(String binding) {
        if (binding == null) {
            return null;
        }
        if (!binding.matches("sc65v1:[0-9a-f]{64}:" + POLICY_HASH + ":[0-9a-f]{64}")) {
            throw new IllegalStateException("Invalid ownerless permission lab binding");
        }
        String[] parts = binding.split(":");
        if (parts[1].matches("0{64}") || parts[3].matches("0{64}")) {
            throw new IllegalStateException("Missing ownerless permission lab commitment");
        }
        for (String variable : new String[] {"SWARMCRAFT_IPC_HOST", "SWARMCRAFT_IPC_PORT",
                "SWARMCRAFT_IPC_TOKEN", "SWARMCRAFT_WORLD_DIR", "SWARMCRAFT_COMPAT_FINGERPRINT",
                "SWARMCRAFT_REQUIRE_AUTHORITY_PERMIT"}) {
            if (System.getenv(variable) != null) {
                throw new IllegalStateException("Ownerless lab cannot attach to a V1 managed runtime");
            }
        }
        return binding;
    }

    public static boolean active() {
        return BINDING != null;
    }

    public static String binding() {
        return BINDING;
    }

    public static boolean ordinary(LevelBasedPermissionSet permissions) {
        return permissions.level() == PermissionLevel.ALL;
    }

    // Check the actual parsed settings before Minecraft opens any listener.
    public static void checkSettings(DedicatedServerProperties properties) {
        if (!active()) {
            return;
        }
        if (!properties.serverIp.equals("127.0.0.1") || !properties.onlineMode
                || !properties.enforceSecureProfile || properties.enableRcon || properties.enableQuery
                || properties.managementServerEnabled || properties.enableJmxMonitoring
                || !properties.whiteList.get() || !properties.enforceWhitelist.get()
                || !ordinary(properties.opPermissions.get()) || !ordinary(properties.functionPermissions)) {
            throw new IllegalStateException("Unsafe ownerless permission lab settings");
        }
        // Do not silently inherit an OP list from a legacy world or a
        // different host. Refuse before Minecraft creates a listener.
        Path ops = Path.of("ops.json");
        if (!Files.notExists(ops, LinkOption.NOFOLLOW_LINKS)) {
            try {
                if (!Files.isRegularFile(ops, LinkOption.NOFOLLOW_LINKS) || Files.size(ops) > 4096) {
                    throw new IllegalStateException("Ownerless permission lab refuses inherited operator list");
                }
                var document = JsonParser.parseString(Files.readString(ops));
                if (!document.isJsonArray() || document.getAsJsonArray().size() != 0) {
                    throw new IllegalStateException("Ownerless permission lab refuses inherited operator list");
                }
            } catch (IOException | com.google.gson.JsonParseException error) {
                throw new IllegalStateException("Ownerless permission lab cannot verify operator list", error);
            }
        }
    }
}
