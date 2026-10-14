package com.example.rustmod;

import com.mojang.logging.LogUtils;
import net.minecraft.core.registries.Registries;
import net.minecraft.server.level.ServerChunkCache;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;

/**
 * Times NoiseBasedChunkGenerator#fillFromNoise (the stage the mod accelerates) with the native path
 * and with pure vanilla, on the same chunks. Only this stage is measured: biomes, surface, carvers,
 * features, structures and light are not included, so the speedup of full world generation is smaller.
 */
public final class Bench {
    private static final Logger LOGGER = LogUtils.getLogger();
    private static final AtomicBoolean RUNNING = HashCheck.RUNNING;
    private static final int WARMUP_CHUNKS = 16;
    private static final int ROUNDS = 3;

    private Bench() {
    }

    public static boolean start(ServerLevel level, int centerX, int centerZ, int chunks, Consumer<String> report) {
        if (!RUNNING.compareAndSet(false, true)) {
            return false;
        }
        Thread thread = new Thread(() -> {
            try {
                for (String line : run(level, centerX, centerZ, chunks)) {
                    report.accept(line);
                }
            } catch (Throwable error) {
                LOGGER.error("Benchmark failed", error);
                report.accept("Benchmark aborted: " + error);
            } finally {
                FillBinding.setForceVanilla(false);
                FillBinding.setHashChecking(false);
                RUNNING.set(false);
            }
        }, "rustmod-bench");
        thread.setDaemon(true);
        thread.start();
        return true;
    }

    private static List<String> run(ServerLevel level, int centerX, int centerZ, int chunks) throws Exception {
        ServerChunkCache cache = level.getChunkSource();
        ChunkGenerator generator = cache.getGenerator();
        RandomState randomState = cache.randomState();
        NativeWorld world = WorldRegistry.get(randomState);
        List<String> lines = new ArrayList<>();
        if (!(generator instanceof NoiseBasedChunkGenerator noiseGenerator)) {
            lines.add("This dimension does not use a noise based generator");
            return lines;
        }
        if (world == null || !world.isEnabled()) {
            lines.add("This dimension has no enabled native world");
            return lines;
        }
        int side = (int) Math.ceil(Math.sqrt(chunks));
        List<ChunkPos> positions = new ArrayList<>();
        for (int index = 0; index < chunks; index++) {
            positions.add(new ChunkPos(centerX + index % side, centerZ + index / side));
        }
        List<ChunkPos> warmup = new ArrayList<>();
        for (int index = 0; index < WARMUP_CHUNKS; index++) {
            warmup.add(new ChunkPos(centerX - 1 - index, centerZ - 1));
        }

        FillBinding.resetDiagnostics();
        FillBinding.setHashChecking(true);
        long nativeBefore = world.nativeSliceCount();
        for (boolean vanilla : new boolean[] {false, true}) {
            FillBinding.setForceVanilla(vanilla);
            sequential(level, noiseGenerator, randomState, warmup);
            parallel(level, noiseGenerator, randomState, warmup);
        }

        double[] nativeSequential = new double[ROUNDS];
        double[] vanillaSequential = new double[ROUNDS];
        double[] nativeParallel = new double[ROUNDS];
        double[] vanillaParallel = new double[ROUNDS];
        for (int round = 0; round < ROUNDS; round++) {
            FillBinding.setForceVanilla(false);
            nativeSequential[round] = sequential(level, noiseGenerator, randomState, positions);
            nativeParallel[round] = parallel(level, noiseGenerator, randomState, positions);
            FillBinding.setForceVanilla(true);
            vanillaSequential[round] = sequential(level, noiseGenerator, randomState, positions);
            vanillaParallel[round] = parallel(level, noiseGenerator, randomState, positions);
        }
        FillBinding.setForceVanilla(false);

        long used = world.nativeSliceCount() - nativeBefore;
        if (used == 0) {
            lines.add("Benchmark inconclusive: the native path produced no slices. " + FillBinding.diagnostics());
            return lines;
        }
        double nativeSeq = median(nativeSequential) / chunks;
        double vanillaSeq = median(vanillaSequential) / chunks;
        double nativePar = median(nativeParallel) / chunks;
        double vanillaPar = median(vanillaParallel) / chunks;
        lines.add(String.format("Benchmark of fillFromNoise on %d chunks, median of %d rounds, ms per chunk:", chunks, ROUNDS));
        lines.add(String.format("  one chunk at a time: native %.2f, vanilla %.2f, speedup x%.2f", nativeSeq, vanillaSeq, vanillaSeq / nativeSeq));
        lines.add(String.format("  all chunks queued:   native %.2f, vanilla %.2f, speedup x%.2f", nativePar, vanillaPar, vanillaPar / nativePar));
        lines.add(String.format("  native slices used: %d, worker threads: %d. Only the noise fill stage is measured.", used, RustBridge.workerCount()));
        LOGGER.info("Benchmark: seq native {} vanilla {}, queued native {} vanilla {} (ms/chunk), {}",
                nativeSeq, vanillaSeq, nativePar, vanillaPar, world.summary());
        return lines;
    }

    private static double sequential(ServerLevel level, NoiseBasedChunkGenerator generator, RandomState randomState,
                                     List<ChunkPos> positions) throws Exception {
        long started = System.nanoTime();
        for (ChunkPos position : positions) {
            fill(level, generator, randomState, position).get();
        }
        return (System.nanoTime() - started) / 1_000_000.0;
    }

    private static double parallel(ServerLevel level, NoiseBasedChunkGenerator generator, RandomState randomState,
                                   List<ChunkPos> positions) throws Exception {
        long started = System.nanoTime();
        List<CompletableFuture<ChunkAccess>> futures = new ArrayList<>(positions.size());
        for (ChunkPos position : positions) {
            futures.add(fill(level, generator, randomState, position));
        }
        for (CompletableFuture<ChunkAccess> future : futures) {
            future.get();
        }
        return (System.nanoTime() - started) / 1_000_000.0;
    }

    private static CompletableFuture<ChunkAccess> fill(ServerLevel level, NoiseBasedChunkGenerator generator,
                                                       RandomState randomState, ChunkPos position) {
        ProtoChunk chunk = new ProtoChunk(position, UpgradeData.EMPTY, level,
                level.registryAccess().registryOrThrow(Registries.BIOME), null);
        return generator.fillFromNoise(Blender.empty(), randomState, level.structureManager(), chunk);
    }

    private static double median(double[] values) {
        double[] sorted = values.clone();
        java.util.Arrays.sort(sorted);
        return sorted[sorted.length / 2];
    }
}
