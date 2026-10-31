package me.aomona.smoke;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

/** Uses the vendor Knot loader, without initializing Minecraft's UI or contacting an account. */
public final class LoaderSmoke {
    public static void main(String[] args) throws Exception {
        Class.forName("net.fabricmc.loader.impl.launch.knot.Knot");
        Class<?> api = Class.forName("net.fabricmc.loader.impl.launch.knot.KnotClassLoaderInterface");
        Class<?> env = Class.forName("net.fabricmc.api.EnvType");
        Method create = api.getDeclaredMethod("create", boolean.class, boolean.class, env,
            Class.forName("net.fabricmc.loader.impl.game.GameProvider"));
        create.setAccessible(true);
        Object delegate = create.invoke(null, false, false, env.getField("CLIENT").get(null), null);
        Method add = api.getDeclaredMethod("addCodeSource", Path.class); add.setAccessible(true);
        for (String path : System.getProperty("java.class.path").split(java.io.File.pathSeparator))
            add.invoke(delegate, Path.of(path));
        Method get = api.getDeclaredMethod("getClassLoader"); get.setAccessible(true);
        ClassLoader loader = (ClassLoader) get.invoke(delegate);
        Thread.currentThread().setContextClassLoader(loader);
        Class<?> smoke = Class.forName("me.aomona.smoke.VendorSmoke", true, loader);
        if (smoke.getClassLoader() != loader) throw new AssertionError("not loaded through Knot");
        smoke.getMethod("main", String[].class).invoke(null, (Object) args);
        System.out.println("KNOT_VENDOR_AUTH_OK");
    }
}
