import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.regex.*;

/** Synthetic IPC fixture; no web browser, account, dialogs or live HTTP requests. */
public class HostBridgeSmoke {
    public static final class Mapper {
        public String writeValueAsString(Object value) {
            Map<?, ?> map = (Map<?, ?>) value;
            // This deliberately narrow fixture serializer accepts only the constants below.
            if (!map.get("path").equals("/worlds") || !map.get("method").equals("POST")
                || !map.get("body").equals("probe") || !map.get("type").equals("realms")) throw new AssertionError();
            return "{\"type\":\"realms\",\"method\":\"POST\",\"path\":\"/worlds\",\"body\":\"probe\",\"headers\":{}}";
        }
        public Object readValue(String json, Class<?> type) {
            Matcher id = Pattern.compile("\"id\":([0-9]+)").matcher(json);
            if (!id.find()) throw new AssertionError();
            Map<String, Object> result = new HashMap<>();
            result.put("id", Long.parseLong(id.group(1)));
            if (json.contains("\"protocol\":1")) result.put("result", Map.of("protocol", 1));
            else {
                if (!json.contains("\"status\":503") || !json.contains("\"body\":\"busy\"")) throw new AssertionError(json);
                result.put("result", Map.of("status", 503, "body", "busy", "headers", Map.of("retry-after", "9")));
            }
            return result;
        }
    }
    public static void main(String[] args) throws Exception {
        Class<?> type = Class.forName("me.aomona.auth.RealmsConnection", true, null);
        HttpURLConnection original = (HttpURLConnection) new URL("https://pc.realms.minecraft.net/worlds").openConnection();
        original.setRequestProperty("Cookie", "sid=game-supplied-must-not-forward");
        HttpURLConnection connection = (HttpURLConnection) type.getConstructor(HttpURLConnection.class, Object.class).newInstance(original, new Mapper());
        connection.setRequestMethod("POST"); connection.setDoOutput(true);
        connection.getOutputStream().write("probe".getBytes(StandardCharsets.UTF_8));
        connection.connect();
        if (connection.getResponseCode() != 503 || !"9".equals(connection.getHeaderField("Retry-After"))) throw new AssertionError();
        if (!new String(connection.getErrorStream().readAllBytes(), StandardCharsets.UTF_8).equals("busy")) throw new AssertionError();
        try { connection.getInputStream(); throw new AssertionError(); } catch (java.io.IOException expected) { }
        connection.disconnect();
        System.out.println("HOST_HTTP_STATUS_BODY_RETRY_OK");
    }
}
