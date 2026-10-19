package me.aomona.sandbox;

import java.io.IOException;
import java.nio.file.Path;
import java.util.*;

/** Opens the requested file inside the AppContainer; it never delegates I/O to the host. */
public final class NativePaths {
    private static Map<String, String> volumes;
    public static void initialize(String library) {
        Map<String, String> parsed = new LinkedHashMap<>();
        for (String entry : System.getProperty("enderpin.windows.volumes", "").split("\\|")) {
            if (entry.length() < 4 || entry.charAt(1) != ':' || entry.charAt(2) != '=')
                throw new IllegalArgumentException("Invalid Windows volume mapping");
            parsed.putIfAbsent(entry.substring(3), entry.substring(0, 2));
        }
        volumes = Collections.unmodifiableMap(parsed);
        System.load(library);
    }
    public static String realPath(Object input) throws IOException {
        String path = ((Path) input).toAbsolutePath().normalize().toString();
        String resolved = finalPath(path);
        for (Map.Entry<String, String> entry : volumes.entrySet()) {
            String device = entry.getKey();
            if (resolved.regionMatches(true, 0, device, 0, device.length())
                && (resolved.length() == device.length() || resolved.charAt(device.length()) == '\\')) {
                String suffix = resolved.substring(device.length());
                return entry.getValue() + (suffix.isEmpty() ? "\\" : suffix);
            }
        }
        if (resolved.startsWith("\\Device\\Mup\\")) return "\\\\" + resolved.substring(12);
        throw new IOException("Resolved Windows volume has no host mapping");
    }
    private static native String finalPath(String path) throws IOException;
    public static int nameMax(String input) throws IOException {
        return maxName(Path.of(input == null ? System.getProperty("user.dir") : input)
            .toAbsolutePath().normalize().toString());
    }
    private static native int maxName(String path) throws IOException;
}
