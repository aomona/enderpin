package me.aomona.auth;

import java.io.IOException;
import java.lang.reflect.*;
import java.net.URL;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.Map;

public final class AuthBridge {
    private static long nextId;
    private static boolean greeted;
    private static final String KEY_MARKER = "MONALAUNCHER_REMOTE_CHAT_KEY:";
    private static final java.util.Set<String> issuedKeys = java.util.concurrent.ConcurrentHashMap.newKeySet();
    private static volatile Object signingClient;
    private static volatile Object signingMapper;
    private AuthBridge() {}

    public static Object request(Object client, URL url, Object body, Class<?> responseType, int method) throws Throwable {
        Field token = client.getClass().getDeclaredField("accessToken"); token.setAccessible(true);
        boolean authenticated = token.get(client) != null;
        String address = url.toExternalForm();
        boolean join = address.equals("https://sessionserver.mojang.com/session/minecraft/join");
        if (!authenticated && !join) {
            try {
                if (method == 2) return client.getClass().getMethod("mona$original$post", URL.class, Object.class, Class.class).invoke(client, url, body, responseType);
                return client.getClass().getMethod(method == 0 ? "mona$original$get" : "mona$original$post", URL.class, Class.class).invoke(client, url, responseType);
            } catch (InvocationTargetException error) { throw error.getCause(); }
        }
        Object mapper = client.getClass().getClassLoader().loadClass("com.mojang.authlib.minecraft.client.ObjectMapper").getMethod("create").invoke(null);
        String command;
        if (join && method == 2 && body != null) {
            String hash = (String) body.getClass().getMethod("serverId").invoke(body);
            if (!hash.matches("-?[a-fA-F0-9]{1,40}")) throw failure(client, 400);
            command = "{\"type\":\"join\",\"server_hash\":\"" + hash + "\"}";
        } else if (address.equals("https://api.minecraftservices.com/player/attributes") && method == 0) {
            command = "{\"type\":\"properties\"}";
        } else if (address.equals("https://api.minecraftservices.com/privacy/blocklist") && method == 0) {
            command = "{\"type\":\"block_list\"}";
        } else if (address.equals("https://api.minecraftservices.com/player/certificates") && method != 0) {
            command = "{\"type\":\"certificate\"}";
        } else if (address.equals("https://api.minecraftservices.com/player/report") && method == 2 && body != null) {
            String report = (String) mapper.getClass().getMethod("writeValueAsString", Object.class).invoke(mapper, body);
            command = "{\"type\":\"report\",\"body\":" + report + "}";
        } else {
            if (!url.getProtocol().equals("https") || !url.getHost().equals("api.minecraftservices.com")) throw failure(client, 403);
            // Additional vanilla services use the same pinned-origin connection broker.
            try {
                if (method == 2) return client.getClass().getMethod("mona$original$post", URL.class, Object.class, Class.class).invoke(client, url, body, responseType);
                return client.getClass().getMethod(method == 0 ? "mona$original$get" : "mona$original$post", URL.class, Class.class).invoke(client, url, responseType);
            } catch (InvocationTargetException error) { throw error.getCause(); }
        }
        Object value;
        try { value = rpc(client, mapper, command); }
        catch (IOException error) { throw failure(client, 503); }
        if (value == null) return null;
        if (address.equals("https://api.minecraftservices.com/player/certificates")) {
            if (!(value instanceof Map<?,?> certificate) || !(certificate.get("keyPair") instanceof Map<?,?> pair)
                || !(pair.get("privateKey") instanceof String marker) || !marker.matches(KEY_MARKER + "[a-f0-9]{64}"))
                throw failure(client, 503);
            signingClient = client; signingMapper = mapper; issuedKeys.add(marker);
        }
        String json = (String) mapper.getClass().getMethod("writeValueAsString", Object.class).invoke(mapper, value);
        return mapper.getClass().getMethod("readValue", String.class, Class.class).invoke(mapper, json, responseType);
    }

    // Authlib 4 joins through YggdrasilAuthenticationService, bypassing MinecraftClient.
    public static void joinLegacy(Object service, String hash) throws Throwable {
        try {
            if (hash == null || !hash.matches("-?[a-fA-F0-9]{1,40}")) throw new IOException("Invalid server hash");
            rpc(null, mapper(service.getClass()), "{\"type\":\"join\",\"server_hash\":\"" + hash + "\"}");
        } catch (Throwable error) {
            throw (Throwable) service.getClass().getClassLoader()
                .loadClass("com.mojang.authlib.exceptions.AuthenticationUnavailableException")
                .getConstructor(String.class).newInstance("Host authentication broker refused server join");
        }
    }

    public static java.net.HttpURLConnection serviceConnection(Object client, URL url) throws Throwable {
        Field token = client.getClass().getDeclaredField("accessToken"); token.setAccessible(true);
        if (token.get(client) == null) {
            Method original = client.getClass().getDeclaredMethod("mona$original$createUrlConnection", URL.class);
            original.setAccessible(true);
            try { return (java.net.HttpURLConnection) original.invoke(client, url); }
            catch (InvocationTargetException error) { throw error.getCause(); }
        }
        return new RealmsConnection(url, mapper(client.getClass()), true);
    }

    static Object mapper(Class<?> owner) throws ReflectiveOperationException {
        return owner.getClassLoader().loadClass("com.mojang.authlib.minecraft.client.ObjectMapper").getMethod("create").invoke(null);
    }

    public static void openUri(Class<?> owner, java.net.URI uri) {
        try {
            Object mapper = mapper(owner);
            String command = (String) mapper.getClass().getMethod("writeValueAsString", Object.class)
                .invoke(mapper, Map.of("type", "desktop_open", "uri", uri.toString()));
            rpc(null, mapper, command);
        } catch (Throwable error) {
            System.err.println("Enderpin could not open the link or folder; check desktop permissions.");
        }
    }

    private static java.util.List<String> importFiles(Class<?> owner, java.util.List<?> paths, boolean multiple) throws Throwable {
        Object mapper = mapper(owner);
        Map<String, Object> request = new java.util.HashMap<>();
        request.put("type", "import_files"); request.put("multiple", multiple);
        if (paths != null) request.put("paths", paths.stream().map(Object::toString).toList());
        String json = (String) mapper.getClass().getMethod("writeValueAsString", Object.class).invoke(mapper, request);
        Object result = rpc(null, mapper, json);
        if (!(result instanceof java.util.List<?> list) || list.stream().anyMatch(item -> !(item instanceof String)))
            throw new IOException("Invalid import response");
        return list.stream().map(String.class::cast).toList();
    }

    public static String chooseFiles(Class<?> owner, boolean multiple) {
        try {
            java.util.List<String> paths = importFiles(owner, null, multiple);
            return paths.isEmpty() ? null : String.join("|", paths);
        } catch (Throwable error) { System.err.println("Enderpin file selection failed; check import permissions and the host file chooser."); return null; }
    }

    public static void filesDrop(Object screen, java.util.List<?> paths) {
        try {
            java.util.List<java.nio.file.Path> imported = importFiles(screen.getClass(), paths, true).stream().map(java.nio.file.Path::of).toList();
            screen.getClass().getMethod("mona$original$onFilesDrop", java.util.List.class).invoke(screen, imported);
        } catch (Throwable error) { System.err.println("Enderpin file import was cancelled or unavailable."); }
    }

    public static Object realmsConnect(Object request) throws Throwable {
        Class<?> base = request.getClass();
        while (base != null) {
            try {
                Method original = base.getDeclaredMethod("mona$original$connect");
                original.setAccessible(true);
                for (Field field : base.getDeclaredFields()) {
                    if (field.getType() != java.net.HttpURLConnection.class) continue;
                    field.setAccessible(true);
                    java.net.HttpURLConnection connection = (java.net.HttpURLConnection) field.get(request);
                    if (!(connection instanceof RealmsConnection)) field.set(request, new RealmsConnection(connection, mapper(base)));
                    try { return original.invoke(request); }
                    catch (InvocationTargetException error) { throw error.getCause(); }
                }
                throw new IOException("Realms connection field missing");
            } catch (NoSuchMethodException ignored) { base = base.getSuperclass(); }
        }
        throw new IOException("Realms adapter missing");
    }

    public static java.security.PrivateKey readPrivateKey(Class<?> crypt, String value) throws Throwable {
        if (issuedKeys.contains(value)) return new RemotePrivateKey(value.substring(KEY_MARKER.length()));
        // Old-launch markers deliberately take the game's normal invalid-cache path and refetch.
        try { return (java.security.PrivateKey) crypt.getMethod("mona$original$readPrivateKey", String.class).invoke(null, value); }
        catch (InvocationTargetException error) { throw error.getCause(); }
    }

    public static String writePrivateKey(Class<?> crypt, java.security.PrivateKey key) throws Throwable {
        if (key instanceof RemotePrivateKey remote) return KEY_MARKER + remote.id;
        try { return (String) crypt.getMethod("mona$original$writePrivateKey", java.security.PrivateKey.class).invoke(null, key); }
        catch (InvocationTargetException error) { throw error.getCause(); }
    }

    static byte[] signChat(String id, byte[] message) throws java.security.SignatureException {
        try {
            Object client = signingClient, mapper = signingMapper;
            if (client == null || mapper == null || !id.matches("[a-f0-9]{64}") || message.length > 6208)
                throw new IOException("Chat signing unavailable");
            String encoded = java.util.Base64.getEncoder().encodeToString(message);
            Object result = rpc(client, mapper, "{\"type\":\"sign\",\"key_id\":\"" + id + "\",\"message\":\"" + encoded + "\"}");
            if (!(result instanceof Map<?,?> data) || !(data.get("signature") instanceof String signature))
                throw new IOException("Chat signature unavailable");
            byte[] bytes = java.util.Base64.getDecoder().decode(signature);
            if (bytes.length != 256) throw new IOException("Chat signature size invalid");
            return bytes;
        } catch (Throwable error) { throw new java.security.SignatureException("Authentication broker refused chat signature"); }
    }

    private static RuntimeException failure(Object client, int status) throws ReflectiveOperationException {
        if (client == null) return new IllegalStateException("Host broker refused operation: " + status);
        return (RuntimeException) client.getClass().getClassLoader()
            .loadClass("com.mojang.authlib.exceptions.MinecraftClientHttpException").getConstructor(int.class).newInstance(status);
    }

    static synchronized Object rpc(Object client, Object mapper, String command) throws Throwable {
        if (!greeted) {
            Object hello = exchange(client, mapper, "{\"type\":\"hello\"}");
            if (!(hello instanceof Map<?,?> data) || !(data.get("protocol") instanceof Number protocol) || protocol.intValue() != 1)
                throw new IOException("Authentication protocol mismatch");
            greeted = true;
            System.setProperty("monalauncher.auth.broker.handshake", "true");
        }
        return exchange(client, mapper, command);
    }

    private static Object exchange(Object client, Object mapper, String command) throws Throwable {
        long id = ++nextId;
        byte[] request = ("{\"id\":" + id + ",\"command\":" + command + "}").getBytes(StandardCharsets.UTF_8);
        byte[] header = ByteBuffer.allocate(4).putInt(request.length).array();
        transfer(header, true); transfer(request, true); transfer(header, false);
        int length = ByteBuffer.wrap(header).getInt();
        if (length < 1 || length > 4 * 1024 * 1024) throw new IOException("Authentication response size invalid");
        byte[] response = new byte[length]; transfer(response, false);
        Map<?,?> parsed = (Map<?,?>) mapper.getClass().getMethod("readValue", String.class, Class.class)
            .invoke(mapper, new String(response, StandardCharsets.UTF_8), Map.class);
        if (!(parsed.get("id") instanceof Number number) || number.longValue() != id) throw new IOException("Authentication response id mismatch");
        if (parsed.containsKey("error")) {
            int status = switch (String.valueOf(parsed.get("error"))) {
                case "invalid_request" -> 400;
                case "unauthorized", "revoked" -> 401;
                case "forbidden", "network_denied" -> 403;
                case "rate_limited" -> 429;
                case "unsupported" -> 501;
                default -> 503;
            };
            throw failure(client, status);
        }
        if (!parsed.containsKey("result")) throw new IOException("Authentication result missing");
        return parsed.get("result");
    }

    private static void transfer(byte[] bytes, boolean writing) throws IOException {
        int offset = 0;
        while (offset < bytes.length) {
            int count = writing ? NativeIO.write(NativeIO.channel(), bytes, offset, bytes.length - offset) : NativeIO.read(NativeIO.channel(), bytes, offset, bytes.length - offset);
            if (count <= 0) throw new IOException("Authentication channel closed");
            offset += count;
        }
    }
}
