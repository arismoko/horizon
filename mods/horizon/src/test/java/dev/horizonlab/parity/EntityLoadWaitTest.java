package dev.horizonlab.parity;

public final class EntityLoadWaitTest {
    public static void main(String[] args) {
        int[] ticks = {0};
        long[] time = {0};
        EntityLoadWait.untilReady(() -> ticks[0]++, () -> ticks[0] >= 410 ? null : "overworld chunk [1, 2]",
            () -> time[0]++, () -> time[0], 500);
        if (ticks[0] != 410) throw new AssertionError("did not wait for readiness");

        ticks[0] = 0;
        time[0] = 0;
        try {
            EntityLoadWait.untilReady(() -> ticks[0]++, () -> "the_nether chunk [3, 4]",
                () -> time[0]++, () -> time[0], 10);
            throw new AssertionError("did not time out");
        } catch (IllegalStateException expected) {
            if (!expected.getMessage().contains("the_nether chunk [3, 4]")) throw new AssertionError(expected);
            if (ticks[0] != 11) throw new AssertionError("did not tick until the deadline");
        }

        ticks[0] = 0;
        EntityLoadWait.untilReady(() -> ticks[0]++, () -> null, () -> {}, () -> 0, 10);
        if (ticks[0] != 6) throw new AssertionError("lost the minimum settling ticks");
    }
}
