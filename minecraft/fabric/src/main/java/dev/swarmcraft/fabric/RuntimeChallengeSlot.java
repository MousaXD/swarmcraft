package dev.swarmcraft.fabric;

import java.time.Duration;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;

/**
 * One bounded outstanding server-thread readiness observation.
 * A refusal is terminal for that request, not a positive runtime proof.
 * Retain the slot until the queued server task actually drains so a stalled
 * Minecraft thread cannot accumulate an unbounded queue of challenges.
 */
final class RuntimeChallengeSlot {
    // The owning Rust supervisor waits 1200ms. Refuse early without changing
    // that deadline, the controller lease, or the positive evidence checks.
    static final long BUDGET_NANOS = Duration.ofMillis(500).toNanos();
    private final AtomicReference<Pending> active = new AtomicReference<>();

    Pending begin(String requestId, long now) {
        Pending pending = new Pending(requestId, now);
        return active.compareAndSet(null, pending) ? pending : null;
    }

    void release(Pending pending) {
        active.compareAndSet(pending, null);
    }

    static String refusal(String requestId) {
        return "ERROR\t" + requestId + "\tRUNTIME_UNAVAILABLE";
    }

    static final class Pending {
        private final String requestId;
        private final long started;
        private final AtomicBoolean completed = new AtomicBoolean();

        Pending(String requestId, long started) {
            this.requestId = requestId;
            this.started = started;
        }

        boolean completed() {
            return completed.get();
        }

        void refuse(Consumer<String> send) {
            if (completed.compareAndSet(false, true)) {
                send.accept(refusal(requestId));
            }
        }

        void finish(String reply, long now, Consumer<String> send) {
            if (now - started >= BUDGET_NANOS) {
                refuse(send);
            } else if (completed.compareAndSet(false, true)) {
                send.accept(reply);
            }
        }
    }
}
