package me.aomona.auth;

import java.lang.instrument.*;
import java.security.ProtectionDomain;

/** Compatibility adapter only: all authorization and secrets remain in Rust. */
public final class AuthAgent {
    public static void premain(String options, Instrumentation instrumentation) {
        if (options == null || options.isEmpty()) throw new IllegalArgumentException("auth bridge native library missing");
        String stage = "bootstrap";
        try {
            java.nio.file.Path bootstrap = java.nio.file.Path.of(options).getParent().resolve("auth-bootstrap.jar");
            instrumentation.appendToBootstrapClassLoaderSearch(new java.util.jar.JarFile(bootstrap.toFile()));
            stage = "native IPC";
            Class.forName("me.aomona.auth.NativeIO", true, null).getMethod("initialize", String.class).invoke(null, options);
            stage = "signature provider";
            Class.forName("me.aomona.auth.RemoteProvider", true, null).getMethod("install").invoke(null);
        } catch (ReflectiveOperationException | java.io.IOException error) {
            Throwable cause = error;
            while (cause instanceof java.lang.reflect.InvocationTargetException wrapped && wrapped.getCause() != null)
                cause = wrapped.getCause();
            String detail = cause.getClass().getSimpleName();
            // Only a fixed native diagnostic with numeric OS status may cross into the log.
            String message = cause.getMessage();
            if (cause instanceof java.io.IOException && message != null
                && message.matches("Authentication IPC prepare failed: win32=[0-9]+ type=[0-9]+")) detail += ": " + message;
            if (cause instanceof UnsatisfiedLinkError && message != null && message.contains("dependent libraries"))
                detail += ": native dependency unavailable";
            if (cause instanceof InternalError && "Error loading java.security file".equals(message))
                detail += ": Java security configuration unavailable";
            // Class/method names identify JDK initialization failures without printing paths,
            // property values or arbitrary exception messages.
            Throwable nested = cause;
            for (int depth = 0; nested != null && depth < 5; depth++, nested = nested.getCause()) {
                if (depth > 0) detail += " > " + nested.getClass().getSimpleName();
                StackTraceElement[] trace = nested.getStackTrace();
                for (int i = 0; i < Math.min(trace.length, 4); i++) {
                    if (trace[i].getClassName().startsWith("java.") || trace[i].getClassName().startsWith("sun."))
                        detail += " [" + trace[i].getClassName() + "." + trace[i].getMethodName() + ":" + trace[i].getLineNumber() + "]";
                }
            }
            throw new IllegalStateException("Authentication IPC initialization failed at " + stage + " (" + detail + ")");
        }
        final String[] mapping;
        try (java.io.InputStream input = AuthAgent.class.getResourceAsStream("/auth-adapters.properties")) {
            java.util.Properties mappings = new java.util.Properties();
            if (input == null) throw new java.io.IOException("authentication mappings missing");
            mappings.load(input);
            String entry = mappings.getProperty(System.getProperty("enderpin.minecraft", ""));
            mapping = entry == null ? null : entry.split("\\|", -1);
            if (mapping != null && mapping.length != 8) throw new java.io.IOException("invalid authentication mapping");
        } catch (java.io.IOException error) { throw new IllegalStateException("Authentication mapping unavailable", error); }
        instrumentation.addTransformer(new ClassFileTransformer() {
            @Override public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                                               ProtectionDomain domain, byte[] bytes) {
                boolean crypt = java.util.Set.of("net/minecraft/util/Crypt", "net/minecraft/class_3515").contains(name)
                    || mapping != null && mapping[4].equals(name);
                boolean desktop = java.util.Set.of("net/minecraft/util/Util$OS", "net/minecraft/Util$OS", "net/minecraft/class_156$class_158").contains(name)
                    || mapping != null && mapping[5].equals(name);
                // 26.3 moved the desktop entrypoint out of Util.OS.
                if (mapping != null && mapping[5].equals("com/mojang/blaze3d/Blaze3D")) desktop = mapping[5].equals(name);
                boolean realms = java.util.Set.of("com/mojang/realmsclient/client/Request", "net/minecraft/class_4346").contains(name)
                    || mapping != null && mapping[6].equals(name);
                boolean fileDialog = "org/lwjgl/util/tinyfd/TinyFileDialogs".equals(name);
                boolean drop = java.util.Set.of("net/minecraft/client/gui/screens/packs/PackSelectionScreen", "net/minecraft/class_5375").contains(name)
                    || mapping != null && mapping[7].equals(name);
                boolean legacySession = mapping != null && mapping[1].equals("authlib-4.0.43.jar")
                    && name.equals("com/mojang/authlib/yggdrasil/YggdrasilMinecraftSessionService");
                if (!legacySession && !crypt && !desktop && !realms && !fileDialog && !drop && !"com/mojang/authlib/minecraft/client/MinecraftClient".equals(name)) return null;
                try {
                    byte[] adapted = legacySession ? MethodAdapter.transformLegacySession(bytes) : fileDialog ? MethodAdapter.transformFileDialog(bytes) : drop ? MethodAdapter.transformDrop(bytes) : desktop ? MethodAdapter.transformDesktop(bytes) : realms ? MethodAdapter.transformRealms(bytes)
                        : crypt ? MethodAdapter.transformPrivateKeys(bytes) : MethodAdapter.transform(bytes);
                    if (crypt) System.setProperty("monalauncher.auth.chat.adapter", "crypt-v1");
                    else System.setProperty("monalauncher.auth.adapter", "authlib-client-v1");
                    return adapted;
                }
                catch (Throwable error) {
                    // Returning the original class after an adapter failure would silently change
                    // behavior. Abort this JVM; never fall back to credential-bearing arguments.
                    System.err.println("Enderpin adapter incompatible: " + name + " (" + error.getClass().getSimpleName() + ")");
                    Runtime.getRuntime().halt(78);
                    return null;
                }
            }
        });
    }
}
