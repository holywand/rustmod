package com.example.rustmod;

import com.mojang.logging.LogUtils;
import net.minecraft.world.level.levelgen.DensityFunction;
import org.slf4j.Logger;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.DoubleBuffer;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

public final class NativeWorld implements AutoCloseable {
    private static final Logger LOGGER = LogUtils.getLogger();
    private static final ThreadLocal<ByteBuffer> GRID_BUFFER = new ThreadLocal<>();
    private static final int PROBE_COUNT = 48;

    private final long handle;
    private final int minY;
    private final int height;
    private final int cellWidth;
    private final int cellHeight;
    private final int slotCount;
    private final int gridLength;
    private final boolean[] beardifierSlots;
    private final ConcurrentHashMap<Integer, Integer> slotByOrder = new ConcurrentHashMap<>();
    private final AtomicBoolean closed = new AtomicBoolean();
    private final AtomicInteger shadowBudget;
    private final AtomicLong shadowSampleCounter = new AtomicLong();
    private final int shadowRate = Math.max(0, Integer.getInteger("rustmod.shadowRate", 128));
    private final int[] probePoints;
    private final AtomicLong nativeSlices = new AtomicLong();
    private final AtomicLong vanillaSlices = new AtomicLong();
    private final AtomicLong shadowChecked = new AtomicLong();
    private final AtomicLong shadowMismatches = new AtomicLong();
    private final String name;
    private volatile boolean enabled = true;
    private volatile String disabledReason = "";
    private volatile double[] rustProbe;

    private NativeWorld(String name, long handle, int minY, int height, int cellWidth, int cellHeight) {
        this.name = name;
        this.handle = handle;
        this.minY = minY;
        this.height = height;
        this.cellWidth = cellWidth;
        this.cellHeight = cellHeight;
        this.slotCount = RustBridge.slotCount(handle);
        this.gridLength = RustBridge.gridLength(handle);
        this.beardifierSlots = new boolean[Math.max(slotCount, 0)];
        for (int slot = 0; slot < beardifierSlots.length; slot++) {
            beardifierSlots[slot] = RustBridge.slotUsesBeardifier(handle, slot) != 0;
        }
        this.shadowBudget = new AtomicInteger(Integer.getInteger("rustmod.shadowChecks", 256));
        this.probePoints = buildProbePoints(minY, height);
    }

    private static int[] buildProbePoints(int minY, int height) {
        int[] points = new int[PROBE_COUNT * 3];
        long state = 0x9E3779B97F4A7C15L;
        for (int probe = 0; probe < PROBE_COUNT; probe++) {
            long scale = probe < 8 ? 512L : (probe < 24 ? 60_000L : 29_000_000L);
            state = mix(state);
            points[probe * 3] = (int) Math.floorMod(state >>> 8, 2 * scale + 1) - (int) scale;
            state = mix(state);
            points[probe * 3 + 1] = minY + (int) Math.floorMod(state >>> 8, (long) height);
            state = mix(state);
            points[probe * 3 + 2] = (int) Math.floorMod(state >>> 8, 2 * scale + 1) - (int) scale;
        }
        return points;
    }

    private static long mix(long value) {
        long z = value + 0x9E3779B97F4A7C15L;
        z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
        z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
        return z ^ (z >>> 31);
    }

    public static NativeWorld create(String name, long seed, String graphJson, int minY, int height, int cellWidth, int cellHeight) {
        if (!RustBridge.isAvailable()) {
            throw new IllegalStateException("native library is not loaded");
        }
        long handle = RustBridge.createWorld(seed, graphJson, minY, height, cellWidth, cellHeight, "final_density");
        if (handle <= 0) {
            throw new IllegalStateException("native world was not created");
        }
        NativeWorld world = new NativeWorld(name, handle, minY, height, cellWidth, cellHeight);
        if (world.slotCount < 0 || world.gridLength < 0) {
            world.close();
            throw new IllegalStateException("native world reported invalid dimensions");
        }
        return world;
    }

    public String name() {
        return name;
    }

    public long handle() {
        return handle;
    }

    public int minY() {
        return minY;
    }

    public int height() {
        return height;
    }

    public int cellWidth() {
        return cellWidth;
    }

    public int cellHeight() {
        return cellHeight;
    }

    public int slotCount() {
        return slotCount;
    }

    public int cornersXZ() {
        return 16 / cellWidth + 1;
    }

    public int cornersY() {
        return height / cellHeight + 1;
    }

    public boolean isEnabled() {
        return enabled && !closed.get();
    }

    public String disabledReason() {
        return disabledReason;
    }

    public void setEnabled(boolean value) {
        if (value) {
            disabledReason = "";
        }
        enabled = value;
    }

    public void disable(String reason) {
        if (enabled) {
            LOGGER.error("Native terrain disabled for {}: {}", name, reason);
        }
        disabledReason = reason;
        enabled = false;
    }

    public boolean usesBeardifier(int slot) {
        return slot < 0 || slot >= beardifierSlots.length || beardifierSlots[slot];
    }

    public int findRoot(String rootName) {
        return RustBridge.findRoot(handle, rootName);
    }

    public double[] evaluate(int root, int[] points) {
        double[] output = new double[points.length / 3];
        int status = RustBridge.evaluate(handle, root, points, output);
        if (status != RustBridge.STATUS_OK) {
            throw new IllegalStateException("native evaluate failed with status " + status);
        }
        return output;
    }

    public DoubleBuffer computeGrid(int chunkX, int chunkZ) {
        int bytes = gridLength * Double.BYTES;
        ByteBuffer buffer = GRID_BUFFER.get();
        if (buffer == null || buffer.capacity() < bytes) {
            buffer = ByteBuffer.allocateDirect(bytes).order(ByteOrder.nativeOrder());
            GRID_BUFFER.set(buffer);
        }
        buffer.clear();
        int status = RustBridge.cornerGrid(handle, chunkX, chunkZ, buffer);
        if (status != RustBridge.STATUS_OK) {
            throw new IllegalStateException("native corner grid failed with status " + status);
        }
        return buffer.asDoubleBuffer();
    }

    public int slotFor(int order, DensityFunction filler) {
        Integer known = slotByOrder.get(order);
        if (known != null) {
            return known;
        }
        int resolved = calibrate(filler);
        Integer previous = slotByOrder.putIfAbsent(order, resolved);
        return previous != null ? previous : resolved;
    }

    private int calibrate(DensityFunction filler) {
        double[] rust = rustProbe;
        if (rust == null) {
            synchronized (this) {
                rust = rustProbe;
                if (rust == null) {
                    rust = new double[slotCount * PROBE_COUNT];
                    int status = RustBridge.probeInterpolated(handle, probePoints, rust);
                    if (status != RustBridge.STATUS_OK) {
                        throw new IllegalStateException("native probe failed with status " + status);
                    }
                    rustProbe = rust;
                }
            }
        }
        double[] vanilla = new double[PROBE_COUNT];
        for (int probe = 0; probe < PROBE_COUNT; probe++) {
            int base = probe * 3;
            vanilla[probe] = filler.compute(new DensityFunction.SinglePointContext(
                    probePoints[base], probePoints[base + 1], probePoints[base + 2]));
        }
        boolean varies = false;
        for (int probe = 1; probe < PROBE_COUNT; probe++) {
            varies |= !sameValue(vanilla[0], vanilla[probe]);
        }
        if (!varies) {
            LOGGER.warn("{}: interpolator output is constant on all probes, falling back to vanilla for it", name);
            return -1;
        }
        int found = -1;
        int bestSlot = -1;
        int bestMatches = -1;
        double bestDelta = Double.POSITIVE_INFINITY;
        for (int slot = 0; slot < slotCount; slot++) {
            int matched = 0;
            double delta = 0;
            for (int probe = 0; probe < PROBE_COUNT; probe++) {
                double expected = vanilla[probe];
                double actual = rust[slot * PROBE_COUNT + probe];
                if (sameValue(expected, actual)) {
                    matched++;
                } else if (!Double.isNaN(expected) && !Double.isNaN(actual)) {
                    delta = Math.max(delta, Math.abs(expected - actual));
                } else {
                    delta = Double.POSITIVE_INFINITY;
                }
            }
            if (matched > bestMatches || (matched == bestMatches && delta < bestDelta)) {
                bestMatches = matched;
                bestSlot = slot;
                bestDelta = delta;
            }
            if (matched == PROBE_COUNT) {
                if (found >= 0) {
                    LOGGER.warn("{}: interpolator matches native slots {} and {}, falling back to vanilla for it", name, found, slot);
                    return -1;
                }
                found = slot;
            }
        }
        if (found < 0) {
            LOGGER.warn("{}: no native slot matches the interpolator (best slot {}: {}/{} probes equal, max delta {}; slots={}), falling back to vanilla for it",
                    name, bestSlot, bestMatches, PROBE_COUNT, bestDelta, slotCount);
        } else {
            LOGGER.info("{}: interpolator bound to native slot {}", name, found);
        }
        return found;
    }

    static boolean sameValue(double first, double second) {
        return first == second || (Double.isNaN(first) && Double.isNaN(second));
    }

    public boolean takeShadowTicket() {
        if (FillBinding.isHashChecking()) {
            return false;
        }
        if (shadowBudget.getAndUpdate(value -> value > 0 ? value - 1 : 0) > 0) {
            return true;
        }
        return shadowRate > 0 && shadowSampleCounter.incrementAndGet() % shadowRate == 0;
    }

    public long nativeSliceCount() {
        return nativeSlices.get();
    }

    public long shadowMismatchCount() {
        return shadowMismatches.get();
    }

    public void countNative() {
        nativeSlices.incrementAndGet();
    }

    public void countVanilla() {
        vanillaSlices.incrementAndGet();
    }

    public void countShadow(boolean mismatch) {
        shadowChecked.incrementAndGet();
        if (mismatch) {
            shadowMismatches.incrementAndGet();
        }
    }

    public String summary() {
        return String.format(
                "%s: %s, slots=%d, native=%d, vanilla=%d, shadow checks=%d, mismatches=%d%s",
                name,
                isEnabled() ? "enabled" : "disabled",
                slotCount,
                nativeSlices.get(),
                vanillaSlices.get(),
                shadowChecked.get(),
                shadowMismatches.get(),
                disabledReason.isEmpty() ? "" : " (" + disabledReason + ")");
    }

    @Override
    public void close() {
        if (closed.compareAndSet(false, true)) {
            RustBridge.destroyWorld(handle);
        }
    }
}
