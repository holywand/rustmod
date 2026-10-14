package com.example.rustmod;

import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.IntegerArgumentType;
import com.mojang.brigadier.context.CommandContext;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.levelgen.DensityFunction;
import net.minecraft.world.level.levelgen.RandomState;

import java.util.List;
import java.util.Map;
import java.util.Random;

public final class RustModCommands {
    private static final int DEFAULT_POINTS = 1024;

    private RustModCommands() {
    }

    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        dispatcher.register(Commands.literal("rustmod")
                .requires(source -> source.hasPermission(2))
                .then(Commands.literal("status").executes(RustModCommands::status))
                .then(Commands.literal("enable").executes(context -> toggle(context, true)))
                .then(Commands.literal("disable").executes(context -> toggle(context, false)))
                .then(Commands.literal("hashcheck")
                        .executes(context -> hashCheck(context, 2))
                        .then(Commands.argument("radius", IntegerArgumentType.integer(0, 6))
                                .executes(context -> hashCheck(context, IntegerArgumentType.getInteger(context, "radius")))))
                .then(Commands.literal("bench")
                        .executes(context -> bench(context, 64))
                        .then(Commands.argument("chunks", IntegerArgumentType.integer(16, 400))
                                .executes(context -> bench(context, IntegerArgumentType.getInteger(context, "chunks")))))
                .then(Commands.literal("verify")
                        .executes(context -> verify(context, DEFAULT_POINTS))
                        .then(Commands.argument("points", IntegerArgumentType.integer(16, 20_000))
                                .executes(context -> verify(context, IntegerArgumentType.getInteger(context, "points"))))));
    }

    private static void reply(CommandContext<CommandSourceStack> context, String message) {
        context.getSource().sendSuccess(() -> Component.literal(message), false);
    }

    private static int status(CommandContext<CommandSourceStack> context) {
        if (!RustBridge.isAvailable()) {
            reply(context, "Native library is not loaded, vanilla generation is active");
            return 0;
        }
        reply(context, "Native library loaded, " + RustBridge.workerCount() + " worker threads, native panics caught: " + RustBridge.panics());
        List<NativeWorld> worlds = WorldRegistry.all();
        if (worlds.isEmpty()) {
            reply(context, "No native worlds registered");
        }
        for (NativeWorld world : worlds) {
            reply(context, world.summary());
        }
        return worlds.size();
    }

    private static int toggle(CommandContext<CommandSourceStack> context, boolean enabled) {
        NativeWorld world = WorldRegistry.get(context.getSource().getLevel());
        if (world == null) {
            reply(context, "This dimension has no native world");
            return 0;
        }
        world.setEnabled(enabled);
        reply(context, world.summary());
        return 1;
    }

    private static int hashCheck(CommandContext<CommandSourceStack> context, int radius) {
        CommandSourceStack source = context.getSource();
        ServerLevel level = source.getLevel();
        int chunkX = Math.floorDiv((int) Math.floor(source.getPosition().x), 16);
        int chunkZ = Math.floorDiv((int) Math.floor(source.getPosition().z), 16);
        boolean started = HashCheck.start(level, chunkX, chunkZ, radius,
                message -> source.getServer().execute(() -> reply(context, message)));
        reply(context, started
                ? "Hash check started on " + (2 * radius + 1) * (2 * radius + 1) + " chunks, the result will follow"
                : "A hash check is already running");
        return started ? 1 : 0;
    }

    private static int bench(CommandContext<CommandSourceStack> context, int chunks) {
        CommandSourceStack source = context.getSource();
        ServerLevel level = source.getLevel();
        int chunkX = Math.floorDiv((int) Math.floor(source.getPosition().x), 16);
        int chunkZ = Math.floorDiv((int) Math.floor(source.getPosition().z), 16);
        boolean started = Bench.start(level, chunkX, chunkZ, chunks,
                message -> source.getServer().execute(() -> reply(context, message)));
        reply(context, started
                ? "Benchmark started on " + chunks + " chunks, takes up to a minute, the game may stutter. Result will follow"
                : "A benchmark or hash check is already running");
        return started ? 1 : 0;
    }

    private static int verify(CommandContext<CommandSourceStack> context, int pointCount) {
        ServerLevel level = context.getSource().getLevel();
        RandomState randomState = level.getChunkSource().randomState();
        NativeWorld world = WorldRegistry.get(randomState);
        if (world == null) {
            reply(context, "This dimension has no native world");
            return 0;
        }

        int[] points = new int[pointCount * 3];
        Random random = new Random(0xC0FFEEL);
        for (int index = 0; index < pointCount; index++) {
            points[index * 3] = random.nextInt(8193) - 4096;
            points[index * 3 + 1] = world.minY() + random.nextInt(world.height());
            points[index * 3 + 2] = random.nextInt(8193) - 4096;
        }

        Map<String, DensityFunction> functions = DensityExporter.routerFunctions(randomState.router());
        int failures = 0;
        for (Map.Entry<String, DensityFunction> entry : functions.entrySet()) {
            int root = world.findRoot(entry.getKey());
            if (root < 0) {
                reply(context, entry.getKey() + ": missing in native graph");
                failures++;
                continue;
            }
            double[] nativeValues;
            try {
                nativeValues = world.evaluate(root, points);
            } catch (RuntimeException error) {
                reply(context, entry.getKey() + ": native evaluation failed: " + error.getMessage());
                failures++;
                continue;
            }
            int exact = 0;
            double maxDelta = 0.0;
            String firstMismatch = "";
            for (int index = 0; index < pointCount; index++) {
                int x = points[index * 3];
                int y = points[index * 3 + 1];
                int z = points[index * 3 + 2];
                double vanilla = entry.getValue().compute(new DensityFunction.SinglePointContext(x, y, z));
                double rust = nativeValues[index];
                if (NativeWorld.sameValue(vanilla, rust)) {
                    exact++;
                } else {
                    maxDelta = Math.max(maxDelta, Math.abs(vanilla - rust));
                    if (firstMismatch.isEmpty()) {
                        firstMismatch = String.format(" first at (%d, %d, %d): vanilla %s, native %s", x, y, z, vanilla, rust);
                    }
                }
            }
            if (exact != pointCount) {
                failures++;
            }
            reply(context, String.format("%s: %d/%d exact, max delta %s%s",
                    entry.getKey(), exact, pointCount, maxDelta, firstMismatch));
        }
        reply(context, failures == 0
                ? "All router functions match vanilla exactly"
                : failures + " router functions differ from vanilla; see the lines above");
        return failures == 0 ? 1 : 0;
    }
}
