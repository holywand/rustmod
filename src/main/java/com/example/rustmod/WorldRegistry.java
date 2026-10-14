package com.example.rustmod;

import com.mojang.logging.LogUtils;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.NoiseSettings;
import net.minecraft.world.level.levelgen.RandomState;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

public final class WorldRegistry {
    private static final Logger LOGGER = LogUtils.getLogger();
    private static final Map<RandomState, NativeWorld> WORLDS = new IdentityHashMap<>();
    private static final Object LOCK = new Object();

    private WorldRegistry() {
    }

    public static NativeWorld get(RandomState randomState) {
        synchronized (LOCK) {
            return WORLDS.get(randomState);
        }
    }

    public static NativeWorld get(ServerLevel level) {
        return get(level.getChunkSource().randomState());
    }

    public static List<NativeWorld> all() {
        synchronized (LOCK) {
            return new ArrayList<>(WORLDS.values());
        }
    }

    public static void install(ServerLevel level) {
        MixinSelfCheck.runOnce();
        if (!RustBridge.isAvailable()) {
            return;
        }
        ChunkGenerator generator = level.getChunkSource().getGenerator();
        if (!(generator instanceof NoiseBasedChunkGenerator noiseGenerator)) {
            return;
        }
        NoiseGeneratorSettings settings = noiseGenerator.generatorSettings().value();
        if (settings.useLegacyRandomSource()) {
            LOGGER.info("{} uses the legacy random source, native terrain skipped", level.dimension().location());
            return;
        }
        NoiseSettings noiseSettings = settings.noiseSettings();
        String name = level.dimension().location().toString();
        long started = System.nanoTime();
        try {
            String document = DensityExporter.export(level.getServer().registryAccess(), settings);
            NativeWorld world = NativeWorld.create(
                    name,
                    level.getSeed(),
                    document,
                    noiseSettings.minY(),
                    noiseSettings.height(),
                    noiseSettings.getCellWidth(),
                    noiseSettings.getCellHeight());
            RandomState randomState = level.getChunkSource().randomState();
            NativeWorld previous;
            synchronized (LOCK) {
                previous = WORLDS.put(randomState, world);
            }
            if (previous != null) {
                previous.close();
            }
            LOGGER.info("Native terrain ready for {} ({} interpolated functions, {} ms)",
                    name, world.slotCount(), (System.nanoTime() - started) / 1_000_000L);
        } catch (Throwable error) {
            LOGGER.error("Native terrain unavailable for {}, vanilla generation stays active", name, error);
        }
    }

    public static void remove(ServerLevel level) {
        NativeWorld world;
        synchronized (LOCK) {
            world = WORLDS.remove(level.getChunkSource().randomState());
        }
        if (world != null) {
            LOGGER.info("Native terrain released for {}: {}", world.name(), world.summary());
            world.close();
        }
    }
}
