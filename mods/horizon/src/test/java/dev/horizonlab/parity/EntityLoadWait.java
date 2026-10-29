package dev.horizonlab.parity;

import java.util.function.LongSupplier;
import java.util.function.Supplier;

/** Wait for asynchronous entity sections while the oracle keeps ticking. */
final class EntityLoadWait {
    static void untilReady(Runnable tick, Supplier<String> unready, Runnable pause,
                           LongSupplier clock, long timeoutNanos) {
        long start = clock.getAsLong();
        int ticks = 0;
        while (true) {
            tick.run();
            ticks++;
            String chunk = unready.get();
            if (chunk == null && ticks >= 6) {
                return;
            }
            if (clock.getAsLong() - start >= timeoutNanos) {
                throw new IllegalStateException("entities around the player did not load: " + chunk);
            }
            pause.run();
        }
    }
}
