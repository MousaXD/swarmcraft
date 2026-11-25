package dev.swarmcraft.probe;

import com.mojang.authlib.GameProfile;
import dev.swarmcraft.fabric.OwnerlessPermissionGuard;
import java.io.IOException;
import java.net.ConnectException;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.util.Optional;
import java.util.UUID;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.Permissions;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.ServerOpListEntry;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.CommandBlockEntity;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

/** Test-only mod; never packaged into the production bridge. */
public final class OwnerlessProbe implements ModInitializer {
    @Override
    public void onInitialize() {
        ServerLifecycleEvents.SERVER_STARTED.register(OwnerlessProbe::probe);
    }

    private static void check(boolean result, String name) {
        if (!result) {
            throw new IllegalStateException("OWNERLESS_PROBE_FAIL " + name);
        }
        System.out.println("OWNERLESS_PROBE_PASS " + name);
    }

    private static boolean listener(int port) {
        try (Socket socket = new Socket()) {
            socket.connect(new InetSocketAddress("127.0.0.1", port), 1000);
            return true;
        } catch (ConnectException refused) {
            return false;
        } catch (IOException error) {
            throw new IllegalStateException("Cannot determine actual listener", error);
        }
    }

    private static void probe(MinecraftServer server) {
        if (!(server instanceof DedicatedServer dedicated)) {
            throw new IllegalStateException("Expected a dedicated server");
        }
        String mode = System.getenv("SC65_PROBE_MODE");
        if ("legacy".equals(mode)) {
            check(!OwnerlessPermissionGuard.active(), "legacy-mode-inactive");
            check(server.createCommandSourceStack().permissions().hasPermission(Permissions.COMMANDS_OWNER),
                "legacy-console-unchanged");
            NameAndId profile = new NameAndId(UUID.nameUUIDFromBytes("sc65-legacy".getBytes(java.nio.charset.StandardCharsets.UTF_8)), "LegacyOp");
            server.getPlayerList().op(profile);
            check(server.getPlayerList().isOp(profile) && !server.getPlayerList().getOps().isEmpty(),
                "legacy-operator-grant-unchanged");
            System.out.println("OWNERLESS_PROBE_COMPLETE legacy");
            server.halt(false);
            return;
        }
        if (!"restricted".equals(mode)) {
            throw new IllegalStateException("Explicit probe mode required");
        }
        check(OwnerlessPermissionGuard.active(), "signed-policy-pin-active");
        check(!dedicated.getProperties().enableRcon && !dedicated.getProperties().managementServerEnabled
            && !dedicated.getProperties().enableJmxMonitoring && !dedicated.getProperties().enableQuery,
            "management-listeners-disabled");
        check(listener(server.getPort()) && !listener(dedicated.getProperties().rconPort)
            && !listener(dedicated.getProperties().managementServerPort), "actual-management-sockets-closed");
        var list = server.getPlayerList();
        check(list.getOps().isEmpty(), "initial-op-list-empty");
        for (String name : new String[] {"Founder", "Successor", "Ordinary"}) {
            UUID uuid = UUID.nameUUIDFromBytes(("sc65-" + name).getBytes(java.nio.charset.StandardCharsets.UTF_8));
            NameAndId profile = new NameAndId(uuid, name);
            ServerPlayer player = new ServerPlayer(server, server.overworld(), new GameProfile(uuid, name),
                ClientInformation.createDefault());
            check(!list.isOp(profile) && OwnerlessPermissionGuard.ordinary(server.getProfilePermissions(profile))
                && !player.permissions().hasPermission(Permissions.COMMANDS_MODERATOR), name + "-ordinary-player");
            CommandSourceStack source = new CommandSourceStack(server, Vec3.ZERO, Vec2.ZERO, server.overworld(),
                player.permissions(), name, Component.literal(name), server, player);
            var root = server.getCommands().getDispatcher().getRoot();
            check(root.getChild("op") != null && !root.getChild("op").canUse(source)
                && !root.getChild("op").canUse(source.withPermission(LevelBasedPermissionSet.OWNER))
                && !source.withPermission(LevelBasedPermissionSet.OWNER).permissions().hasPermission(Permissions.COMMANDS_GAMEMASTER),
                name + "-op-and-elevation-denied");
            check(root.getChild("help").canUse(source), name + "-ordinary-command-available");
            check(java.util.stream.Stream.of("gamerule", "give", "gamemode", "function", "stop")
                .allMatch(command -> root.getChild(command) != null
                    && !root.getChild(command).canUse(source.withPermission(LevelBasedPermissionSet.OWNER))),
                name + "-protected-commands-denied");
            server.getCommands().performPrefixedCommand(source, "op " + name);
            list.op(profile);
            list.op(profile, Optional.of(LevelBasedPermissionSet.OWNER), Optional.of(true));
            check(!list.getOps().add(new ServerOpListEntry(profile, LevelBasedPermissionSet.OWNER, true))
                && !list.isOp(profile) && list.getOps().isEmpty() && !list.canBypassPlayerLimit(profile),
                name + "-direct-op-and-bypass-denied");
            check(list.canPlayerLogin(new InetSocketAddress("127.0.0.1", 25565), profile) != null,
                name + "-live-admission-gated");
        }
        var console = server.createCommandSourceStack();
        check(!console.permissions().hasPermission(Permissions.COMMANDS_OWNER), "console-elevation-denied");
        dedicated.handleConsoleInput("op Founder", console);
        check(dedicated.runCommand("op Successor").contains("disabled"), "rcon-command-denied");
        dedicated.setOperatorUserPermissions(LevelBasedPermissionSet.OWNER);
        list.setAllowCommandsForAllPlayers(true);
        check(OwnerlessPermissionGuard.ordinary(dedicated.operatorUserPermissions())
            && !list.isAllowCommandsForAllPlayers() && list.getOps().isEmpty(), "policy-change-denied");
        var block = new CommandBlockEntity(BlockPos.ZERO, Blocks.COMMAND_BLOCK.defaultBlockState()).getCommandBlock();
        block.setCommand("op Founder");
        check(!block.performCommand(server.overworld()), "command-block-execution-denied");
        check(list.getOps().isEmpty(), "final-op-list-empty");
        System.out.println("OWNERLESS_PROBE_COMPLETE restricted");
        server.halt(false);
    }
}
