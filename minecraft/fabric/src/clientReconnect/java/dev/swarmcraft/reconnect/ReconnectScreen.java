package dev.swarmcraft.reconnect;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.ConnectScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.network.chat.Component;

import java.util.concurrent.ExecutionException;
import java.util.concurrent.FutureTask;
import java.time.Duration;

/** Explicit one-shot reconnect request. All UI decisions run on the client thread. */
final class ReconnectScreen extends Screen {
    private final Screen parent;
    private final ReconnectPolicy.Config config;
    private final ReconnectPolicy.AttemptWindow window = new ReconnectPolicy.AttemptWindow(System.nanoTime());
    private final long firstProbeAt;
    private FutureTask<RouteProbe.Result> pending;
    private String status = "Checking a fresh SwarmCraft route...";
    private boolean finished;

    ReconnectScreen(Screen parent, ReconnectPolicy.Config config, boolean automatic) {
        super(Component.literal("SwarmCraft reconnect"));
        this.parent = parent;
        this.config = config;
        // Explicitly configured automatic mode still presents a visible
        // cancellation opportunity before launching a local probe.
        firstProbeAt = System.nanoTime() + (automatic ? Duration.ofSeconds(2).toNanos() : 0);
        if (automatic) {
            status = "Automatic reconnect enabled. Cancel to stay offline.";
        }
    }

    @Override
    protected void init() {
        var cancel = addRenderableWidget(Button.builder(Component.literal("Cancel / Back"), b -> onClose())
                .bounds(width / 2 - 90, height / 2 + 40, 180, 20).build());
        setInitialFocus(cancel);
    }

    @Override
    public void tick() {
        if (finished || minecraft.screen != this) {
            return;
        }
        long now = System.nanoTime();
        if (!minecraft.allowsMultiplayer() || minecraft.getConnection() != null || window.expired(now)) {
            stop();
            status = "Reconnect stopped. No new connection opened.";
            return;
        }
        if (now - firstProbeAt < 0) {
            return;
        }
        if (pending != null && pending.isDone()) {
            RouteProbe.Result result = null;
            try {
                result = pending.get();
            } catch (ExecutionException | InterruptedException | java.util.concurrent.CancellationException unavailable) {
                // No valid result can authorize a connection.
            }
            pending = null;
            now = System.nanoTime();
            if (result != null && result.proof().isPresent()
                    && result.proof().get().usable(config, result.requestedAt(), System.currentTimeMillis())
                    && minecraft.screen == this && minecraft.getConnection() == null && minecraft.allowsMultiplayer()
                    && window.authorizeConnection(now)) {
                // Vanilla starts a new login using the existing player
                // session. The gateway still verifies every incoming
                // connection independently; the proof is not a permit.
                finished = true;
                ServerData server = new ServerData("SwarmCraft", config.gateway(), ServerData.Type.OTHER);
                ConnectScreen.startConnecting(parent, minecraft, new ServerAddress("127.0.0.1", config.port()),
                        server, false, null);
                return;
            }
            window.failed(now);
            status = "No fresh trusted route yet. Waiting to retry.";
        }
        if (window.exhausted(now)) {
            stop();
            status = "No verified route. Reconnect stopped.";
            return;
        }
        if (pending == null && window.begin(now)) {
            long budget = Math.min(ReconnectPolicy.PROBE_NS, window.remaining(now));
            status = "Checking trusted route (" + window.attempts() + "/" + ReconnectPolicy.MAX_ATTEMPTS + ")...";
            pending = new FutureTask<>(() -> RouteProbe.check(config, budget));
            Thread.ofVirtual().name("swarmcraft-reconnect-probe").start(pending);
        }
    }

    private void stop() {
        finished = true;
        window.stop();
        if (pending != null) {
            pending.cancel(true);
            pending = null;
        }
    }

    @Override
    public void onClose() {
        stop();
        minecraft.setScreen(parent);
    }

    @Override
    public void removed() {
        stop();
    }

    @Override
    public void extractRenderState(GuiGraphicsExtractor graphics, int mouseX, int mouseY, float partialTick) {
        super.extractRenderState(graphics, mouseX, mouseY, partialTick);
        graphics.centeredText(font, title, width / 2, height / 2 - 48, 0xFFFFFFFF);
        graphics.centeredText(font, status, width / 2, height / 2 - 20, 0xFFFFFFFF);
        graphics.centeredText(font, "Only the same verified local gateway may be used.",
                width / 2, height / 2 - 4, 0xFFBBBBBB);
        graphics.centeredText(font, "At most 3 checks in 45 seconds; then manual reconnect.",
                width / 2, height / 2 + 12, 0xFFBBBBBB);
    }
}
