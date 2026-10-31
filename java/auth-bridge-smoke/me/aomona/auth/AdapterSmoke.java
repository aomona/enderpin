package me.aomona.auth;
import java.nio.file.*;
import java.util.zip.*;
/** Verifies real pinned vendor class signatures without loading the game or using an account. */
public class AdapterSmoke {
    public static void main(String[] args) throws Exception {
        for (int i = 0; i < args.length; i += 3) {
            String mode = args[i], jar = args[i + 1], name = args[i + 2];
            try (ZipFile zip = new ZipFile(jar)) {
                byte[] source = zip.getInputStream(zip.getEntry(name + ".class")).readAllBytes();
                byte[] output = switch (mode) {
                    case "legacy-session" -> MethodAdapter.transformLegacySession(source);
                    case "auth" -> MethodAdapter.transform(source);
                    case "crypt" -> MethodAdapter.transformPrivateKeys(source);
                    case "desktop" -> MethodAdapter.transformDesktop(source);
                    case "realms" -> MethodAdapter.transformRealms(source);
                    case "file" -> MethodAdapter.transformFileDialog(source);
                    case "drop" -> MethodAdapter.transformDrop(source);
                    default -> throw new IllegalArgumentException();
                };
                if (output.length <= source.length) throw new AssertionError(name);
                System.out.println("PASS " + mode + " " + name);
            }
        }
    }
}
