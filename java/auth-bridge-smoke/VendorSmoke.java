package me.aomona.smoke;
import java.lang.reflect.*;
import java.net.*;
import java.security.*;
import java.util.*;

/** Exercises transformed official authlib/game classes against synthetic host IPC, never a real token. */
public final class VendorSmoke {
    public static void main(String[] args) throws Exception {
        ClassLoader loader = VendorSmoke.class.getClassLoader();
        Class<?> clientType = Class.forName("com.mojang.authlib.minecraft.client.MinecraftClient", true, loader);
        if (clientType.getClassLoader() != loader) throw new AssertionError("authlib escaped target loader");
        clientType.getDeclaredMethod("mona$original$createUrlConnection", URL.class);
        Object client = clientType.getConstructor(String.class, java.net.Proxy.class).newInstance("placeholder", java.net.Proxy.NO_PROXY);
        Method get = clientType.getMethod("get", URL.class, Class.class);
        Method post = clientType.getMethod("post", URL.class, Object.class, Class.class);
        String namespace = System.getProperty("enderpin.minecraft").equals("26.3") ? "services" : "yggdrasil";
        Class<?> joinType = Class.forName("com.mojang.authlib." + namespace + ".request.JoinMinecraftServerRequest", true, loader);
        if (joinType.isRecord()) {
            Object join = joinType.getConstructor(String.class, UUID.class, String.class)
                .newInstance("placeholder", new UUID(0, 0), "123abc");
            post.invoke(client, new URL("https://sessionserver.mojang.com/session/minecraft/join"), join, Void.class);
        } else {
            Class<?> session = Class.forName("com.mojang.authlib.yggdrasil.YggdrasilMinecraftSessionService", true, loader);
            Class<?> envType = Class.forName("com.mojang.authlib.yggdrasil.YggdrasilEnvironment", true, loader);
            Object environment = envType.getMethod("getEnvironment").invoke(envType.getField("PROD").get(null));
            Constructor<?> constructor = session.getDeclaredConstructors()[0]; constructor.setAccessible(true);
            Object service = constructor.newInstance(null, environment);
            Class<?> profileType = Class.forName("com.mojang.authlib.GameProfile", true, loader);
            Object profile = profileType.getConstructor(UUID.class, String.class).newInstance(new UUID(0, 0), "fixture");
            Method joinServer = session.getMethod("joinServer", profileType, String.class, String.class);
            joinServer.invoke(service, profile, "placeholder", "123abc");
            try {
                joinServer.invoke(service, profile, "placeholder", "not-a-server-hash");
                throw new AssertionError("invalid server hash accepted");
            } catch (InvocationTargetException expected) {
                if (!expected.getCause().getClass().getName().equals("com.mojang.authlib.exceptions.AuthenticationUnavailableException")) throw expected;
            }
            try {
                joinServer.invoke(service, profile, "placeholder", "dead");
                throw new AssertionError("broker denial bypassed");
            } catch (InvocationTargetException expected) {
                if (!expected.getCause().getClass().getName().equals("com.mojang.authlib.exceptions.AuthenticationUnavailableException")) throw expected;
            }
        }
        Class<?> attributes = Class.forName("com.mojang.authlib." + namespace + ".response.UserAttributesResponse", true, loader);
        Object properties = get.invoke(client, new URL("https://api.minecraftservices.com/player/attributes"), attributes);
        if (properties == null) throw new AssertionError("attributes missing");
        get.invoke(client, new URL("https://api.minecraftservices.com/privacy/blocklist"), Map.class);
        Map<?, ?> cert = (Map<?, ?>) post.invoke(client, new URL("https://api.minecraftservices.com/player/certificates"), null, Map.class);
        String marker = (String) ((Map<?, ?>) cert.get("keyPair")).get("privateKey");
        Class<?> crypt = Class.forName(args[0].replace('/', '.'), true, loader);
        PrivateKey key = null;
        for (Method method : crypt.getDeclaredMethods()) {
            if (!method.getName().startsWith("mona$") && Modifier.isStatic(method.getModifiers())
                && method.getReturnType() == PrivateKey.class && Arrays.equals(method.getParameterTypes(), new Class<?>[]{String.class}))
                key = (PrivateKey) method.invoke(null, marker);
        }
        if (key == null || key.getEncoded() != null || key.getFormat() != null) throw new AssertionError("private key exported");
        boolean encodedMarker = false;
        for (Method method : crypt.getDeclaredMethods()) {
            if (!method.getName().startsWith("mona$") && Modifier.isStatic(method.getModifiers())
                && method.getReturnType() == String.class && Arrays.equals(method.getParameterTypes(), new Class<?>[]{PrivateKey.class}))
                encodedMarker = marker.equals(method.invoke(null, key));
        }
        if (!encodedMarker) throw new AssertionError("remote key marker not preserved");
        Signature signer = Signature.getInstance("SHA256withRSA"); signer.initSign(key); signer.update(new byte[]{1, 2, 3});
        if (signer.sign().length != 256) throw new AssertionError("remote signing failed");
        for (int i = 1; i < args.length; i++) {
            Class<?> type = Class.forName(args[i].replace('/', '.'), false, loader);
            if (Arrays.stream(type.getDeclaredMethods()).noneMatch(m -> m.getName().startsWith("mona$original$")))
                throw new AssertionError("unadapted game class " + args[i]);
        }
        System.out.println("VENDOR_JOIN_ATTRIBUTES_CERTIFICATE_REMOTE_KEY_OK");
    }
}
