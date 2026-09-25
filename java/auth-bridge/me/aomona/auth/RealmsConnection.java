package me.aomona.auth;

import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

/** Preserves Realms' status/body/retry semantics; credentials are attached only by the host. */
public final class RealmsConnection extends HttpURLConnection {
    private final Object mapper;
    private final boolean service;
    private final ByteArrayOutputStream body = new ByteArrayOutputStream();
    private Map<?, ?> reply;
    public RealmsConnection(HttpURLConnection original, Object mapper) throws IOException {
        this(original.getURL(), mapper, false);
        original.getRequestProperties().forEach((key, values) -> values.forEach(value -> addRequestProperty(key, value)));
    }
    public RealmsConnection(URL url, Object mapper, boolean service) throws IOException {
        super(url);
        this.mapper = mapper;
        this.service = service;
        if (!url.getProtocol().equals("https") || !url.getHost().equals(service ? "api.minecraftservices.com" : "pc.realms.minecraft.net")
            || (url.getPort() != -1 && url.getPort() != 443) || url.getUserInfo() != null)
            throw new IOException("Unsupported Realms origin");
    }
    @Override public void connect() { }
    @Override public void disconnect() { }
    @Override public boolean usingProxy() { return false; }
    @Override public OutputStream getOutputStream() {
        return new OutputStream() {
            @Override public void write(int value) throws IOException {
                if (body.size() >= 512 * 1024) throw new IOException("Realms request too large");
                body.write(value);
            }
        };
    }
    private void fetch() throws IOException {
        if (reply != null) return;
        try {
            Map<String, Object> request = new HashMap<>();
            request.put("type", service ? "game_services" : "realms"); request.put("method", getRequestMethod());
            request.put("path", url.getFile()); request.put("body", body.toString(StandardCharsets.UTF_8));
            Map<String, String> headers = new HashMap<>();
            // Cookies (including sid) and arbitrary headers are never forwarded from the game.
            for (String name : List.of("Is-Prerelease", "Minecraft-Version", "Minecraft-Protocol-Version", "If-None-Match", "If-Match")) {
                String value = getRequestProperty(name);
                if (value != null) headers.put(name, value);
            }
            request.put("headers", headers);
            String json = (String) mapper.getClass().getMethod("writeValueAsString", Object.class).invoke(mapper, request);
            Object result = AuthBridge.rpc(null, mapper, json);
            if (!(result instanceof Map<?, ?> map) || !(map.get("status") instanceof Number))
                throw new IOException("Invalid Realms response");
            reply = map;
            responseCode = ((Number) reply.get("status")).intValue();
        } catch (Throwable error) { throw new IOException("Realms broker request failed"); }
    }
    @Override public int getResponseCode() throws IOException { fetch(); return responseCode; }
    @Override public InputStream getInputStream() throws IOException {
        fetch();
        if (responseCode >= 400) throw new IOException("Realms HTTP " + responseCode);
        return stream();
    }
    @Override public InputStream getErrorStream() {
        try { fetch(); return responseCode >= 400 ? stream() : null; }
        catch (IOException error) { return null; }
    }
    private InputStream stream() { return new ByteArrayInputStream(String.valueOf(reply.get("body")).getBytes(StandardCharsets.UTF_8)); }
    @Override public String getHeaderField(String name) {
        try {
            fetch();
            Object headers = reply.get("headers");
            return headers instanceof Map<?, ?> map ? (String) map.get(name.toLowerCase(Locale.ROOT)) : null;
        } catch (IOException error) { return null; }
    }
}
