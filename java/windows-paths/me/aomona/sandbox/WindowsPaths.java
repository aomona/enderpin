package me.aomona.sandbox;

import java.lang.instrument.*;
import java.security.ProtectionDomain;
import java.util.*;

/** Preserve filesystem isolation when the JDK resolves an already accessible path. */
public final class WindowsPaths {
    public static void premain(String ignored, Instrumentation instrumentation) throws Exception {
        java.nio.file.Path jar = java.nio.file.Path.of(WindowsPaths.class.getProtectionDomain()
            .getCodeSource().getLocation().toURI());
        instrumentation.appendToBootstrapClassLoaderSearch(new java.util.jar.JarFile(
            jar.getParent().resolve("windows-paths-bootstrap.jar").toFile()));
        Class<?> nativePaths = Class.forName("me.aomona.sandbox.NativePaths", true, null);
        nativePaths.getMethod("initialize", String.class).invoke(null,
            jar.getParent().resolve("windows-paths.dll").toString());
        instrumentation.redefineModule(Object.class.getModule(), Set.of(nativePaths.getModule()),
            Map.of(), Map.of(), Set.of(), Map.of());
        instrumentation.addTransformer(new ClassFileTransformer() {
            @Override public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                                               ProtectionDomain domain, byte[] bytes) {
                boolean realPath = "sun/nio/fs/WindowsLinkSupport".equals(name);
                if (!realPath && !"java/io/WinNTFileSystem".equals(name)) return null;
                try { return PathAdapter.transform(bytes, realPath); }
                catch (Throwable error) {
                    System.err.println("Enderpin Windows path adapter incompatible: " + error.getClass().getSimpleName());
                    Runtime.getRuntime().halt(78);
                    return null;
                }
            }
        }, true);
        instrumentation.retransformClasses(Class.forName("sun.nio.fs.WindowsLinkSupport", false, null),
            Class.forName("java.io.WinNTFileSystem", false, null));
    }
}
