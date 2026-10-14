package com.example.rustmod;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.serialization.JsonOps;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.RegistryOps;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.level.levelgen.DensityFunction;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.NoiseRouter;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

import java.util.LinkedHashMap;
import java.util.Map;

public final class DensityExporter {
    private DensityExporter() {
    }

    public static String export(RegistryAccess registryAccess, NoiseGeneratorSettings settings) {
        RegistryOps<JsonElement> ops = RegistryOps.create(JsonOps.INSTANCE, registryAccess);

        JsonObject noises = new JsonObject();
        Registry<NormalNoise.NoiseParameters> noiseRegistry = registryAccess.registryOrThrow(Registries.NOISE);
        for (Map.Entry<ResourceKey<NormalNoise.NoiseParameters>, NormalNoise.NoiseParameters> entry : noiseRegistry.entrySet()) {
            JsonElement encoded = NormalNoise.NoiseParameters.DIRECT_CODEC
                    .encodeStart(JsonOps.INSTANCE, entry.getValue())
                    .getOrThrow();
            noises.add(entry.getKey().location().toString(), encoded);
        }

        JsonObject functions = new JsonObject();
        Registry<DensityFunction> functionRegistry = registryAccess.registryOrThrow(Registries.DENSITY_FUNCTION);
        for (Map.Entry<ResourceKey<DensityFunction>, DensityFunction> entry : functionRegistry.entrySet()) {
            JsonElement encoded = DensityFunction.DIRECT_CODEC
                    .encodeStart(ops, entry.getValue())
                    .getOrThrow();
            functions.add(entry.getKey().location().toString(), encoded);
        }

        JsonObject router = new JsonObject();
        for (Map.Entry<String, DensityFunction> entry : routerFunctions(settings.noiseRouter()).entrySet()) {
            JsonElement encoded = DensityFunction.HOLDER_HELPER_CODEC
                    .encodeStart(ops, entry.getValue())
                    .getOrThrow();
            router.add(entry.getKey(), encoded);
        }

        JsonObject document = new JsonObject();
        document.add("noises", noises);
        document.add("functions", functions);
        document.add("router", router);
        return document.toString();
    }

    public static Map<String, DensityFunction> routerFunctions(NoiseRouter router) {
        Map<String, DensityFunction> entries = new LinkedHashMap<>();
        entries.put("barrier", router.barrierNoise());
        entries.put("fluid_level_floodedness", router.fluidLevelFloodednessNoise());
        entries.put("fluid_level_spread", router.fluidLevelSpreadNoise());
        entries.put("lava", router.lavaNoise());
        entries.put("temperature", router.temperature());
        entries.put("vegetation", router.vegetation());
        entries.put("continents", router.continents());
        entries.put("erosion", router.erosion());
        entries.put("depth", router.depth());
        entries.put("ridges", router.ridges());
        entries.put("initial_density_without_jaggedness", router.initialDensityWithoutJaggedness());
        entries.put("final_density", router.finalDensity());
        entries.put("vein_toggle", router.veinToggle());
        entries.put("vein_ridged", router.veinRidged());
        entries.put("vein_gap", router.veinGap());
        return entries;
    }
}
