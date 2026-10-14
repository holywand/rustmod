package com.example.rustmod;

import com.mojang.logging.LogUtils;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.server.level.ServerChunkCache;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;
import org.slf4j.Logger;

import java.util.function.Consumer;

public final class HashCheck {
    private static final Logger LOGGER = LogUtils.getLogger();
    static final java.util.concurrent.atomic.AtomicBoolean RUNNING = new java.util.concurrent.atomic.AtomicBoolean();

    private HashCheck() {
    }

    public static boolean start(ServerLevel level, int centerX, int centerZ, int radius, Consumer<String> report) {
        if (!RUNNING.compareAndSet(false, true)) {
            return false;
        }
        Thread thread = new Thread(() -> {
            try {
                report.accept(run(level, centerX, centerZ, radius));
            } catch (Throwable error) {
                LOGGER.error("Hash check failed", error);
                report.accept("Hash check aborted: " + error);
            } finally {
                FillBinding.setForceVanilla(false);
                FillBinding.setHashChecking(false);
                RUNNING.set(false);
            }
        }, "rustmod-hashcheck");
        thread.setDaemon(true);
        thread.start();
        return true;
    }

    private static String run(ServerLevel level, int centerX, int centerZ, int radius) throws Exception {
        ServerChunkCache cache = level.getChunkSource();
        ChunkGenerator generator = cache.getGenerator();
        RandomState randomState = cache.randomState();
        NativeWorld world = WorldRegistry.get(randomState);
        if (!(generator instanceof NoiseBasedChunkGenerator noiseGenerator)) {
            return "This dimension does not use a noise based generator";
        }
        if (world == null || !world.isEnabled()) {
            return "This dimension has no enabled native world";
        }
        long nativeBefore = world.nativeSliceCount();
        int compared = 0;
        int different = 0;
        String first = "";
        FillBinding.resetDiagnostics();
        FillBinding.setHashChecking(true);
        for (int dx = -radius; dx <= radius; dx++) {
            for (int dz = -radius; dz <= radius; dz++) {
                ChunkPos position = new ChunkPos(centerX + dx, centerZ + dz);
                FillBinding.setForceVanilla(false);
                long nativeHash = hash(level, noiseGenerator, randomState, position);
                FillBinding.setForceVanilla(true);
                long vanillaHash = hash(level, noiseGenerator, randomState, position);
                FillBinding.setForceVanilla(false);
                compared++;
                if (nativeHash != vanillaHash) {
                    different++;
                    if (first.isEmpty()) {
                        first = String.format(" first at chunk (%d, %d): native %016x, vanilla %016x",
                                position.x, position.z, nativeHash, vanillaHash);
                    }
                }
            }
        }
        long used = world.nativeSliceCount() - nativeBefore;
        if (different > 0) {
            world.disable("hash check mismatch:" + first);
        }
        if (used == 0) {
            String diagnostics = FillBinding.diagnostics();
            LOGGER.warn("Hash check: no native slices. {} | {}", diagnostics, world.summary());
            return "Hash check inconclusive: the native path produced no slices during the run. " + diagnostics;
        }
        return different == 0
                ? String.format("Hash check passed: %d chunks identical, %d native slices used", compared, used)
                : String.format("Hash check FAILED: %d of %d chunks differ.%s", different, compared, first);
    }

    private static long hash(ServerLevel level, NoiseBasedChunkGenerator generator, RandomState randomState, ChunkPos position) throws Exception {
        ProtoChunk chunk = new ProtoChunk(position, UpgradeData.EMPTY, level,
                level.registryAccess().registryOrThrow(Registries.BIOME), null);
        ChunkAccess filled = generator.fillFromNoise(Blender.empty(), randomState, level.structureManager(), chunk).get();
        long hash = 0xCBF29CE484222325L;
        BlockPos.MutableBlockPos cursor = new BlockPos.MutableBlockPos();
        int minY = filled.getMinBuildHeight();
        int maxY = filled.getMaxBuildHeight();
        for (int x = 0; x < 16; x++) {
            for (int z = 0; z < 16; z++) {
                for (int y = minY; y < maxY; y++) {
                    cursor.set(position.getMinBlockX() + x, y, position.getMinBlockZ() + z);
                    int id = Block.BLOCK_STATE_REGISTRY.getId(filled.getBlockState(cursor));
                    hash = (hash ^ id) * 0x100000001B3L;
                }
                hash = (hash ^ filled.getHeight(Heightmap.Types.WORLD_SURFACE_WG, x, z)) * 0x100000001B3L;
                hash = (hash ^ filled.getHeight(Heightmap.Types.OCEAN_FLOOR_WG, x, z)) * 0x100000001B3L;
            }
        }
        return hash;
    }
}
