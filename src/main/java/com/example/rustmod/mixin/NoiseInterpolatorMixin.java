package com.example.rustmod.mixin;

import com.example.rustmod.FillBinding;
import net.minecraft.world.level.levelgen.DensityFunction;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * In 1.21.1 NoiseChunk fills each interpolation slice one Z column at a time through
 * NoiseInterpolator#fillArray(double[], ContextProvider). The column position is read from the
 * context provider, so NoiseChunk keeps running its own loop and counters.
 */
@Mixin(targets = "net.minecraft.world.level.levelgen.NoiseChunk$NoiseInterpolator")
public abstract class NoiseInterpolatorMixin {
    @Shadow
    @Final
    private DensityFunction noiseFiller;

    @Unique
    private double[] rustmod$expected;
    @Unique
    private int rustmod$blockX;
    @Unique
    private int rustmod$blockZ;

    @Inject(method = "fillArray([DLnet/minecraft/world/level/levelgen/DensityFunction$ContextProvider;)V",
            at = @At("HEAD"), cancellable = true, require = 0)
    private void rustmod$fillArrayHead(double[] array, DensityFunction.ContextProvider provider, CallbackInfo ci) {
        this.rustmod$expected = null;
        FillBinding.diag(FillBinding.SLICE_CALLS);
        FillBinding binding = FillBinding.current();
        if (binding == null) {
            FillBinding.diag(FillBinding.SLICE_NO_BINDING);
            return;
        }
        DensityFunction.FunctionContext context = provider.forIndex(0);
        int blockX = context.blockX();
        int blockZ = context.blockZ();
        double[] rust = binding.rustColumn(this, this.noiseFiller, array.length, blockX, blockZ);
        if (rust == null) {
            return;
        }
        if (binding.world().takeShadowTicket()) {
            this.rustmod$expected = rust;
            this.rustmod$blockX = blockX;
            this.rustmod$blockZ = blockZ;
            return;
        }
        System.arraycopy(rust, 0, array, 0, rust.length);
        binding.world().countNative();
        ci.cancel();
    }

    @Inject(method = "fillArray([DLnet/minecraft/world/level/levelgen/DensityFunction$ContextProvider;)V",
            at = @At("RETURN"), require = 0)
    private void rustmod$fillArrayReturn(double[] array, DensityFunction.ContextProvider provider, CallbackInfo ci) {
        double[] expected = this.rustmod$expected;
        if (expected == null) {
            return;
        }
        this.rustmod$expected = null;
        FillBinding binding = FillBinding.current();
        if (binding != null) {
            binding.compare(expected, array, this.rustmod$blockX, this.rustmod$blockZ);
        }
    }
}
