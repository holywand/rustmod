package com.example.rustmod.mixin;

import com.example.rustmod.FillBinding;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(NoiseBasedChunkGenerator.class)
public abstract class NoiseBasedChunkGeneratorMixin {
    private static final String DO_FILL =
            "doFill(Lnet/minecraft/world/level/levelgen/blending/Blender;"
                    + "Lnet/minecraft/world/level/StructureManager;"
                    + "Lnet/minecraft/world/level/levelgen/RandomState;"
                    + "Lnet/minecraft/world/level/chunk/ChunkAccess;II)"
                    + "Lnet/minecraft/world/level/chunk/ChunkAccess;";

    @Inject(method = DO_FILL, at = @At("HEAD"), require = 0)
    private void rustmod$beginFill(Blender blender, StructureManager structureManager, RandomState randomState,
                                   ChunkAccess chunk, int minCellY, int cellCountY,
                                   CallbackInfoReturnable<ChunkAccess> cir) {
        FillBinding.end();
        try {
            FillBinding binding = FillBinding.forFill(blender, randomState, chunk, minCellY, cellCountY);
            if (binding != null) {
                FillBinding.begin(binding);
            }
        } catch (Throwable ignored) {
            FillBinding.end();
        }
    }

    @Inject(method = DO_FILL, at = @At("RETURN"), require = 0)
    private void rustmod$endFill(Blender blender, StructureManager structureManager, RandomState randomState,
                                 ChunkAccess chunk, int minCellY, int cellCountY,
                                 CallbackInfoReturnable<ChunkAccess> cir) {
        FillBinding.end();
    }
}
