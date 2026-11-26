package dev.swarmcraft.reconnect;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientLoginConnectionEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayConnectionEvents;
import net.fabricmc.fabric.api.client.screen.v1.ScreenEvents;
import net.fabricmc.fabric.api.client.screen.v1.Screens;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.DisconnectedScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.network.chat.Component;
import org.slf4j.LoggerFactory;

import java.time.Duration;

/** The opt-in is local, per disconnection, and consumes exactly one eligible session. */
public final class SwarmCraftReconnect implements ClientModInitializer {
    private static final long OFFER_NS = Duration.ofSeconds(5).toNanos();
    private ReconnectPolicy.Config config;
    private ClientPacketListener joined;
    private long offerUntil;
    private Screen offered;
    private Button button;
    private boolean autoPending;

    @Override
    public void onInitializeClient() {
        try {
            config = ReconnectPolicy.Config.load(FabricLoader.getInstance().getConfigDir()
                    .resolve("swarmcraft-reconnect.json"));
        } catch (Exception disabled) {
            // Never log local paths, world identity, or proof data.
            LoggerFactory.getLogger("SwarmCraftReconnect").info(
                    "SwarmCraft reconnect disabled: configure one trusted local gateway and CLI to enable its opt-in button.");
            return;
        }
        ClientLoginConnectionEvents.INIT.register((handler, client) -> client.execute(this::clear));
        ClientPlayConnectionEvents.JOIN.register((handler, sender, client) -> client.execute(() -> {
            clear();
            ServerData server = handler.getServerData();
            if (!client.hasSingleplayerServer() && server != null && server.type() == ServerData.Type.OTHER
                    && config.matchesJoinedGateway(server.ip, handler.getConnection().getRemoteAddress(),
                            handler.getConnection().isMemoryConnection())) {
                joined = handler;
            }
        }));
        ClientPlayConnectionEvents.DISCONNECT.register((handler, client) -> client.execute(() -> {
            if (joined == handler) {
                joined = null;
                offerUntil = System.nanoTime() + OFFER_NS;
            }
        }));
        ScreenEvents.AFTER_INIT.register((client, screen, width, height) -> {
            if (screen == offered) {
                addButton(client, screen);
            } else if (canOffer(client, screen)) {
                offer(client, screen);
            }
        });
        ClientTickEvents.END_CLIENT_TICK.register(client -> {
            if (offered != null && client.screen != offered) {
                offered = null;
                button = null;
                autoPending = false;
            }
            if (canOffer(client, client.screen)) {
                offer(client, client.screen);
            }
            if (offerUntil != 0 && (offerUntil - System.nanoTime() <= 0
                    || (client.getConnection() == null && !(client.screen instanceof DisconnectedScreen)))) {
                // A normal logout or another screen cannot arm a later
                // unrelated failed login, even inside the short window.
                offerUntil = 0;
            }
            if (autoPending && offered == client.screen && client.getConnection() == null) {
                startReconnect(client, offered, true);
            }
        });
    }

    private void clear() {
        joined = null;
        offerUntil = 0;
        offered = null;
        button = null;
        autoPending = false;
    }

    private boolean canOffer(Minecraft client, Screen screen) {
        return offered == null && offerUntil != 0 && offerUntil - System.nanoTime() > 0
                && screen instanceof DisconnectedScreen && client.getConnection() == null
                && client.allowsMultiplayer();
    }

    private void offer(Minecraft client, Screen screen) {
        offered = screen;
        offerUntil = 0;
        autoPending = config.autoReconnect();
        addButton(client, screen);
    }

    private void startReconnect(Minecraft client, Screen screen, boolean automatic) {
        if (offered != screen || client.screen != screen || !(screen instanceof DisconnectedScreen)
                || client.getConnection() != null || !client.allowsMultiplayer()) {
            return;
        }
        if (button != null) {
            button.active = false;
        }
        clear();
        client.setScreen(new ReconnectScreen(screen, config, automatic));
    }

    private void addButton(Minecraft client, Screen screen) {
        if (client.screen != screen || !client.allowsMultiplayer()) {
            return;
        }
        var widgets = Screens.getWidgets(screen);
        if (button != null && widgets.contains(button)) {
            return;
        }
        // Keep the vanilla disconnect reason and controls. Use an empty
        // top/bottom edge rather than replacing a vanilla action.
        int width = Math.min(220, screen.width - 12);
        if (width < 160) {
            return;
        }
        int x = (screen.width - width) / 2;
        int y = -1;
        for (int position : new int[]{screen.height - 26, 6}) {
            if (position >= 0 && widgets.stream().noneMatch(widget ->
                    widget.getX() < x + width && widget.getX() + widget.getWidth() > x
                    && widget.getY() < position + 20 && widget.getY() + widget.getHeight() > position)) {
                y = position;
                break;
            }
        }
        if (y < 0) {
            return;
        }
        button = Button.builder(Component.literal("Reconnect with SwarmCraft"), pressed -> {
            startReconnect(client, screen, false);
        }).bounds(x, y, width, 20).build();
        widgets.add(button);
    }
}
