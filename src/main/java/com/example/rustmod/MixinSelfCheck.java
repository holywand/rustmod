package com.example.rustmod;

import com.mojang.logging.LogUtils;
import org.slf4j.Logger;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicBoolean;

/** Logs, once, whether the mixins were applied and what the targeted classes really look like. */
public final class MixinSelfCheck {
    private static final Logger LOGGER = LogUtils.getLogger();
    private static final AtomicBoolean DONE = new AtomicBoolean();

    private MixinSelfCheck() {
    }

    public static void runOnce() {
        if (!DONE.compareAndSet(false, true)) {
            return;
        }
        describe("net.minecraft.world.level.levelgen.NoiseChunk$NoiseInterpolator", "rustmod$expected");
        describe("net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator", null);
    }

    private static void describe(String className, String appliedMarker) {
        try {
            Class<?> type = Class.forName(className, false, MixinSelfCheck.class.getClassLoader());
            boolean applied = false;
            StringBuilder fields = new StringBuilder();
            for (Field field : type.getDeclaredFields()) {
                if (field.getName().equals(appliedMarker)) {
                    applied = true;
                }
                fields.append(field.getType().getSimpleName()).append(' ').append(field.getName()).append("; ");
            }
            StringBuilder methods = new StringBuilder();
            for (Method method : type.getDeclaredMethods()) {
                String name = method.getName();
                if (name.contains("fill") || name.contains("Fill") || name.contains("lice") || name.startsWith("rustmod")) {
                    methods.append(name).append(Arrays.toString(method.getParameterTypes())).append(" -> ")
                            .append(method.getReturnType().getSimpleName()).append("; ");
                }
            }
            LOGGER.info("MixinSelfCheck {}: marker field {}, fields: {}", className,
                    appliedMarker == null ? "n/a" : (applied ? "PRESENT (mixin applied)" : "MISSING (mixin NOT applied)"), fields);
            LOGGER.info("MixinSelfCheck {}: methods: {}", className, methods);
        } catch (Throwable error) {
            LOGGER.warn("MixinSelfCheck {} failed: {}", className, error.toString());
        }
    }
}
