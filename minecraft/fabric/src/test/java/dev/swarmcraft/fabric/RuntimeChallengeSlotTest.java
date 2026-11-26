package dev.swarmcraft.fabric;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/** Pure policy and real two-thread race tests; no Minecraft launch. */
public final class RuntimeChallengeSlotTest {
    private static int assertions;

    private static void check(boolean condition, String reason) {
        assertions++;
        if (!condition) throw new AssertionError(reason);
    }

    public static void main(String[] args) throws Exception {
        var slot = new RuntimeChallengeSlot();
        var replies = new ArrayList<String>();
        long start = 100;
        var first = slot.begin("1", start);
        check(first != null && !first.completed(), "one pending request");
        check(slot.begin("2", start) == null, "one queued server task");
        first.finish("RUNTIME_READY\t1\tproof", start + 1, replies::add);
        first.refuse(replies::add);
        check(replies.equals(List.of("RUNTIME_READY\t1\tproof")), "one successful response");
        check(slot.begin("2", start) == null, "retain until task drains");
        slot.release(first);
        var expired = slot.begin("2", start);
        check(expired != null, "fresh request after drain");
        expired.finish("RUNTIME_READY\t2\tlate", start + RuntimeChallengeSlot.BUDGET_NANOS, replies::add);
        check(replies.getLast().equals(RuntimeChallengeSlot.refusal("2")), "late success becomes denial");
        expired.refuse(replies::add);
        check(replies.size() == 2, "late task cannot duplicate reply");
        slot.release(expired);

        // Real independent timer and stalled server-thread simulation.
        var stalled = slot.begin("3", System.nanoTime());
        var observed = Collections.synchronizedList(new ArrayList<String>());
        var entered = new CountDownLatch(1);
        var release = new CountDownLatch(1);
        Thread game = Thread.ofPlatform().start(() -> {
            entered.countDown();
            try {
                if (!release.await(3, TimeUnit.SECONDS)) throw new AssertionError("test release missing");
                stalled.finish("RUNTIME_READY\t3\tlate", System.nanoTime(), observed::add);
            } catch (InterruptedException error) {
                Thread.currentThread().interrupt();
            } finally {
                slot.release(stalled);
            }
        });
        try (var timer = new java.util.concurrent.ScheduledThreadPoolExecutor(1)) {
            check(entered.await(1, TimeUnit.SECONDS), "server task started");
            timer.schedule(() -> stalled.refuse(observed::add), 30, TimeUnit.MILLISECONDS).get(1, TimeUnit.SECONDS);
            check(observed.equals(List.of(RuntimeChallengeSlot.refusal("3"))), "timer emits correlated denial");
            for (int i = 0; i < 100; i++) {
                check(slot.begin("queued-" + i, System.nanoTime()) == null, "no queue amplification");
            }
            release.countDown();
            game.join(1000);
            check(!game.isAlive() && observed.size() == 1, "late server callback suppressed");
        } finally {
            release.countDown();
            game.join(1000);
        }
        var fresh = slot.begin("4", System.nanoTime());
        check(fresh != null, "responsive server accepts new challenge");
        slot.release(stalled);
        check(slot.begin("5", System.nanoTime()) == null, "stale release cannot retire replacement");
        fresh.finish("RUNTIME_READY\t4\tnew", System.nanoTime(), observed::add);
        check(observed.equals(List.of(RuntimeChallengeSlot.refusal("3"), "RUNTIME_READY\t4\tnew")),
                "fresh response follows explicit refusal");
        slot.release(fresh);
        System.out.println("Runtime challenge slot: " + assertions + " assertions passed");
    }
}
