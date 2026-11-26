package dev.swarmcraft.reconnect;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.TimeUnit;

/** Bounded subprocess on a background thread; no shell, cached output or raw host fallback. */
final class RouteProbe {
    record Result(Optional<ReconnectPolicy.Proof> proof, long requestedAt) {}

    static Result check(ReconnectPolicy.Config config, long budgetNs) {
        long requestedAt = System.currentTimeMillis();
        try {
            byte[] output = run(config.command(), budgetNs);
            return new Result(ReconnectPolicy.decode(output, config, requestedAt, System.currentTimeMillis()), requestedAt);
        } catch (IOException | RuntimeException unavailable) {
            return new Result(Optional.empty(), requestedAt);
        } catch (InterruptedException cancelled) {
            Thread.currentThread().interrupt();
            return new Result(Optional.empty(), requestedAt);
        }
    }

    static byte[] run(List<String> command, long budgetNs) throws IOException, InterruptedException {
        if (budgetNs <= 0 || budgetNs > ReconnectPolicy.PROBE_NS) {
            throw new IOException("Invalid route probe budget");
        }
        if (Thread.currentThread().isInterrupted()) {
            throw new InterruptedException();
        }
        long deadline = System.nanoTime() + budgetNs;
        // The selected native sidecar is trusted by the local operator.
        // It has no interactive input and stderr is never sent to the game.
        Process process = new ProcessBuilder(command).redirectError(ProcessBuilder.Redirect.DISCARD).start();
        try (var input = process.getInputStream()) {
            process.getOutputStream().close();
            var output = new ByteArrayOutputStream();
            byte[] buffer = new byte[1024];
            while (true) {
                if (Thread.currentThread().isInterrupted()) {
                    throw new InterruptedException();
                }
                if (deadline - System.nanoTime() <= 0) {
                    throw new IOException("Live route probe timed out");
                }
                int available = input.available();
                if (available > 0) {
                    int count = input.read(buffer, 0, Math.min(available, buffer.length));
                    if (count > 0) {
                        if (output.size() + count > ReconnectPolicy.MAX_DOCUMENT_BYTES) {
                            throw new IOException("Live route report exceeds limit");
                        }
                        output.write(buffer, 0, count);
                    }
                    continue;
                }
                if (!process.isAlive()) {
                    // The trusted CLI has exited; drain remaining bytes.
                    // Never wait for an inherited pipe from another process.
                    if (input.available() > 0) {
                        continue;
                    }
                    if (process.exitValue() != 0) {
                        throw new IOException("Live route probe unavailable");
                    }
                    return output.toByteArray();
                }
                process.waitFor(Math.min(deadline - System.nanoTime(), 25_000_000L), TimeUnit.NANOSECONDS);
            }
        } finally {
            if (process.isAlive()) {
                process.destroyForcibly();
            }
        }
    }

    private RouteProbe() {}
}
