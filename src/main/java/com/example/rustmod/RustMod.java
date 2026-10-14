package com.example.rustmod;

import com.mojang.logging.LogUtils;
import net.minecraft.server.level.ServerLevel;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
import net.neoforged.neoforge.event.level.LevelEvent;
import org.slf4j.Logger;

@Mod(RustMod.MOD_ID)
public final class RustMod {
    public static final String MOD_ID = "rustmod";

    private static final Logger LOGGER = LogUtils.getLogger();

    public RustMod() {
        if (RustBridge.isAvailable()) {
            LOGGER.info("Rust terrain optimizer active");
        } else {
            LOGGER.warn("Rust terrain optimizer inactive, vanilla generation in use");
        }
        NeoForge.EVENT_BUS.addListener(RustMod::onLevelLoad);
        NeoForge.EVENT_BUS.addListener(RustMod::onLevelUnload);
        NeoForge.EVENT_BUS.addListener(RustMod::onRegisterCommands);
    }

    private static void onLevelLoad(LevelEvent.Load event) {
        if (event.getLevel() instanceof ServerLevel level) {
            WorldRegistry.install(level);
        }
    }

    private static void onLevelUnload(LevelEvent.Unload event) {
        if (event.getLevel() instanceof ServerLevel level) {
            WorldRegistry.remove(level);
        }
    }

    private static void onRegisterCommands(RegisterCommandsEvent event) {
        RustModCommands.register(event.getDispatcher());
    }
}
