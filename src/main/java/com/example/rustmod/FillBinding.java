package com.example.rustmod;

import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.levelgen.DensityFunction;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;

import java.nio.DoubleBuffer;
import java.util.IdentityHashMap;
import java.util.concurrent.atomic.AtomicLongArray;

public final class FillBinding {
    private static final ThreadLocal<FillBinding> CURRENT = new ThreadLocal<>();
    private static volatile boolean forceVanilla;
    private static volatile boolean hashChecking;

    public static final int FILL_CALLS = 0;
    public static final int REJ_FORCED = 1;
    public static final int REJ_BLENDER = 2;
    public static final int REJ_NO_WORLD = 3;
    public static final int REJ_HEIGHT = 4;
    public static final int REJ_CELLS = 5;
    public static final int BOUND = 6;
    public static final int SLICE_CALLS = 7;
    public static final int SLICE_NO_BINDING = 8;
    public static final int SLICE_BAD_SHAPE = 9;
    public static final int SLICE_BAD_CORNER = 10;
    public static final int SLICE_NO_SLOT = 11;
    public static final int SLICE_BEARDIFIER = 12;
    private static final String[] DIAG_NAMES = {"doFill", "rej:forceVanilla", "rej:blender", "rej:noWorld",
            "rej:height", "rej:cells", "bound", "fillArray", "slice:noBinding", "slice:badShape",
            "slice:badCorner", "slice:noSlot", "slice:beardifier"};
    private static final AtomicLongArray DIAG = new AtomicLongArray(DIAG_NAMES.length);

    public static void diag(int index) {
        DIAG.incrementAndGet(index);
    }

    public static void resetDiagnostics() {
        for (int i = 0; i < DIAG.length(); i++) {
            DIAG.set(i, 0);
        }
    }

    public static String diagnostics() {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < DIAG.length(); i++) {
            if (i > 0) {
                out.append(", ");
            }
            out.append(DIAG_NAMES[i]).append('=').append(DIAG.get(i));
        }
        return out.toString();
    }

    private final NativeWorld world;
    private final int chunkX;
    private final int chunkZ;
    private final int firstCellX;
    private final int firstCellZ;
    private final IdentityHashMap<Object, Integer> order = new IdentityHashMap<>();
    private DoubleBuffer grid;

    private FillBinding(NativeWorld world, int chunkX, int chunkZ) {
        this.world = world;
        this.chunkX = chunkX;
        this.chunkZ = chunkZ;
        this.firstCellX = Math.floorDiv(chunkX * 16, world.cellWidth());
        this.firstCellZ = Math.floorDiv(chunkZ * 16, world.cellWidth());
    }

    public static void setForceVanilla(boolean value) {
        forceVanilla = value;
    }

    public static void setHashChecking(boolean value) {
        hashChecking = value;
    }

    public static boolean isHashChecking() {
        return hashChecking;
    }

    public static FillBinding current() {
        return CURRENT.get();
    }

    public static void begin(FillBinding binding) {
        CURRENT.set(binding);
    }

    public static void end() {
        CURRENT.remove();
    }

    public static FillBinding forFill(Blender blender, RandomState randomState, ChunkAccess chunk, int minCellY, int cellCountY) {
        diag(FILL_CALLS);
        if (forceVanilla) {
            diag(REJ_FORCED);
            return null;
        }
        if (blender != Blender.empty()) {
            diag(REJ_BLENDER);
            return null;
        }
        NativeWorld world = WorldRegistry.get(randomState);
        if (world == null || !world.isEnabled()) {
            diag(REJ_NO_WORLD);
            return null;
        }
        if (chunk.getMinBuildHeight() != world.minY() || chunk.getHeight() != world.height()) {
            diag(REJ_HEIGHT);
            return null;
        }
        if (minCellY != Math.floorDiv(world.minY(), world.cellHeight())
                || cellCountY != world.height() / world.cellHeight()) {
            diag(REJ_CELLS);
            return null;
        }
        diag(BOUND);
        ChunkPos position = chunk.getPos();
        return new FillBinding(world, position.x, position.z);
    }

    public NativeWorld world() {
        return world;
    }

    /**
     * Returns the native values for one corner column (all Y corners at the block column blockX, blockZ),
     * or null when vanilla has to compute it.
     */
    public double[] rustColumn(Object interpolator, DensityFunction filler, int length, int blockX, int blockZ) {
        try {
            if (!world.isEnabled()) {
                return null;
            }
            int cornersXZ = world.cornersXZ();
            int cornersY = world.cornersY();
            if (length != cornersY) {
                diag(SLICE_BAD_SHAPE);
                world.countVanilla();
                return null;
            }
            int cornerX = Math.floorDiv(blockX, world.cellWidth()) - firstCellX;
            int cornerZ = Math.floorDiv(blockZ, world.cellWidth()) - firstCellZ;
            if (cornerX < 0 || cornerX >= cornersXZ || cornerZ < 0 || cornerZ >= cornersXZ) {
                diag(SLICE_BAD_CORNER);
                world.countVanilla();
                return null;
            }
            int slot = world.slotFor(orderOf(interpolator), filler);
            if (slot < 0 || world.usesBeardifier(slot)) {
                diag(slot < 0 ? SLICE_NO_SLOT : SLICE_BEARDIFIER);
                world.countVanilla();
                return null;
            }
            if (grid == null) {
                grid = world.computeGrid(chunkX, chunkZ);
            }
            double[] result = new double[cornersY];
            int row = ((slot * cornersXZ + cornerX) * cornersXZ + cornerZ) * cornersY;
            for (int y = 0; y < cornersY; y++) {
                result[y] = grid.get(row + y);
            }
            return result;
        } catch (Throwable error) {
            world.disable("native column failed: " + error);
            return null;
        }
    }

    public void compare(double[] expected, double[] actual, int blockX, int blockZ) {
        int mismatchAt = -1;
        for (int y = 0; y < actual.length; y++) {
            if (!NativeWorld.sameValue(expected[y], actual[y])) {
                mismatchAt = y;
                break;
            }
        }
        world.countShadow(mismatchAt >= 0);
        if (mismatchAt >= 0) {
            world.disable(String.format("shadow check mismatch at chunk (%d, %d) block x=%d z=%d corner y=%d: native %s, vanilla %s",
                    chunkX, chunkZ, blockX, blockZ, mismatchAt, expected[mismatchAt], actual[mismatchAt]));
        }
    }

    private int orderOf(Object interpolator) {
        Integer known = order.get(interpolator);
        if (known == null) {
            known = order.size();
            order.put(interpolator, known);
        }
        return known;
    }
}
