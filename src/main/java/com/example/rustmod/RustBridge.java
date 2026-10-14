package com.example.rustmod;

import com.mojang.logging.LogUtils;
import org.slf4j.Logger;

import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

public final class RustBridge {
    public static final int STATUS_OK = 0;
    public static final int STATUS_PANIC = 4;

    private static final Logger LOGGER = LogUtils.getLogger();
    private static final String LIBRARY_NAME = "rustmod";
    private static final boolean LOADED;

    static {
        LOADED = initialize();
    }

    private RustBridge() {
    }

    static native long createWorld(long seed, String graphJson, int minY, int height, int cellWidth, int cellHeight, String rootName);

    static native boolean destroyWorld(long handle);

    static native int workerThreads();

    static native boolean configureThreads(int count);

    static native long panicCount();

    static native int findRoot(long handle, String name);

    static native int fillChunk(long handle, int chunkX, int chunkZ, ByteBuffer output);

    static native int evaluate(long handle, int root, int[] points, double[] output);

    static native int slotCount(long handle);

    static native int gridLength(long handle);

    static native int slotUsesBeardifier(long handle, int slot);

    static native int cornerGrid(long handle, int chunkX, int chunkZ, ByteBuffer output);

    static native int probeInterpolated(long handle, int[] points, double[] output);

    public static boolean isAvailable() {
        return LOADED;
    }

    public static long panics() {
        return LOADED ? panicCount() : 0L;
    }

    public static int workerCount() {
        return LOADED ? workerThreads() : 0;
    }

    private static boolean initialize() {
        if (Boolean.getBoolean("rustmod.disable")) {
            LOGGER.info("Native backend disabled by -Drustmod.disable=true");
            return false;
        }
        if (loadFromLibraryPath() && verifyLinkage()) {
            return true;
        }
        if (loadBundled() && verifyLinkage()) {
            return true;
        }
        LOGGER.error("Native library '{}' is unavailable, vanilla terrain generation stays active", LIBRARY_NAME);
        return false;
    }

    private static boolean loadFromLibraryPath() {
        try {
            System.loadLibrary(LIBRARY_NAME);
            LOGGER.info("Loaded native library '{}' from java.library.path", LIBRARY_NAME);
            return true;
        } catch (UnsatisfiedLinkError | SecurityException error) {
            LOGGER.debug("System.loadLibrary('{}') failed, trying the bundled copy", LIBRARY_NAME, error);
            return false;
        }
    }

    private static boolean loadBundled() {
        String fileName = System.mapLibraryName(LIBRARY_NAME);
        String resource = "/natives/" + platformDirectory() + "/" + fileName;
        try (InputStream stream = RustBridge.class.getResourceAsStream(resource)) {
            if (stream == null) {
                LOGGER.warn("No bundled native library at {}", resource);
                return false;
            }
            Path directory = Files.createTempDirectory("rustmod-native");
            Path target = directory.resolve(fileName);
            Files.copy(stream, target, StandardCopyOption.REPLACE_EXISTING);
            target.toFile().deleteOnExit();
            directory.toFile().deleteOnExit();
            System.load(target.toAbsolutePath().toString());
            LOGGER.info("Loaded bundled native library from {}", resource);
            return true;
        } catch (UnsatisfiedLinkError | IOException | SecurityException error) {
            LOGGER.error("Failed to load bundled native library {}", resource, error);
            return false;
        }
    }

    private static boolean verifyLinkage() {
        try {
            int requested = Integer.getInteger("rustmod.threads", 0);
            if (requested > 0 && !configureThreads(requested)) {
                LOGGER.warn("Could not apply -Drustmod.threads={}", requested);
            }
            int threads = workerThreads();
            if (threads <= 0) {
                LOGGER.error("Native worker pool failed to start");
                return false;
            }
            LOGGER.info("Native terrain backend ready with {} worker threads", threads);
            return true;
        } catch (UnsatisfiedLinkError error) {
            LOGGER.error("Loaded native library does not export the expected JNI symbols", error);
            return false;
        }
    }

    private static String platformDirectory() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);
        String osPart;
        if (os.contains("win")) {
            osPart = "windows";
        } else if (os.contains("mac") || os.contains("darwin")) {
            osPart = "macos";
        } else {
            osPart = "linux";
        }
        String archPart = (arch.contains("aarch64") || arch.contains("arm64")) ? "aarch64" : "x86_64";
        return osPart + "-" + archPart;
    }
}
