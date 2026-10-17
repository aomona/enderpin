package me.aomona.auth;

import java.io.*;
import java.util.*;

/** Replaces three straight-line entrypoints without embedding a second ASM on Fabric's classpath. */
final class MethodAdapter {
    private final ByteArrayOutputStream constants = new ByteArrayOutputStream();
    private final DataOutputStream cp = new DataOutputStream(constants);
    private final Map<Integer, String> utf8 = new HashMap<>();
    private int count;

    static byte[] transform(byte[] source) throws IOException { return new MethodAdapter().adapt(source, 0); }

    static byte[] transformPrivateKeys(byte[] source) throws IOException { return new MethodAdapter().adapt(source, 1); }

    static byte[] transformDesktop(byte[] source) throws IOException { return new MethodAdapter().adapt(source, 2); }
    static byte[] transformRealms(byte[] source) throws IOException { return new MethodAdapter().adapt(source, 3); }

    static byte[] transformFileDialog(byte[] source) throws IOException { return new MethodAdapter().adapt(source, 4); }
    static byte[] transformDrop(byte[] source) throws IOException { return new MethodAdapter().adapt(source, 5); }

    private byte[] adapt(byte[] source, int mode) throws IOException {
        boolean crypt = mode == 1;
        DataInputStream in = new DataInputStream(new ByteArrayInputStream(source));
        int magic = in.readInt();
        if (magic != 0xcafebabe) throw new IOException("invalid class");
        int minor = in.readUnsignedShort(), major = in.readUnsignedShort();
        count = in.readUnsignedShort();
        for (int i = 1; i < count; i++) {
            int tag = in.readUnsignedByte(); cp.writeByte(tag);
            switch (tag) {
                case 1: String text = in.readUTF(); cp.writeUTF(text); utf8.put(i, text); break;
                case 3: case 4: case 9: case 10: case 11: case 12: case 17: case 18: cp.write(in.readNBytes(4)); break;
                case 5: case 6: cp.write(in.readNBytes(8)); i++; break;
                case 7: case 8: case 16: case 19: case 20: cp.write(in.readNBytes(2)); break;
                case 15: cp.write(in.readNBytes(3)); break;
                default: throw new IOException("unsupported constant");
            }
        }
        ByteArrayOutputStream body = new ByteArrayOutputStream();
        DataOutputStream out = new DataOutputStream(body);
        byte[] classHeader = in.readNBytes(6); out.write(classHeader);
        int thisClass = (classHeader[2] & 255) << 8 | (classHeader[3] & 255);
        int interfaces = in.readUnsignedShort(); out.writeShort(interfaces); out.write(in.readNBytes(interfaces * 2));
        int fields = in.readUnsignedShort(); out.writeShort(fields);
        for (int i = 0; i < fields; i++) copyMember(in, out);
        int methods = in.readUnsignedShort();
        List<byte[]> originals = new ArrayList<>(), wrappers = new ArrayList<>();
        int bridge = methodRef("me/aomona/auth/AuthBridge", "request", "(Ljava/lang/Object;Ljava/net/URL;Ljava/lang/Object;Ljava/lang/Class;I)Ljava/lang/Object;");
        int readKey = methodRef("me/aomona/auth/AuthBridge", "readPrivateKey", "(Ljava/lang/Class;Ljava/lang/String;)Ljava/security/PrivateKey;");
        int writeKey = methodRef("me/aomona/auth/AuthBridge", "writePrivateKey", "(Ljava/lang/Class;Ljava/security/PrivateKey;)Ljava/lang/String;");
        int desktop = methodRef("me/aomona/auth/AuthBridge", "openUri", "(Ljava/lang/Class;Ljava/net/URI;)V");
        int realms = methodRef("me/aomona/auth/AuthBridge", "realmsConnect", "(Ljava/lang/Object;)Ljava/lang/Object;");
        int choose = methodRef("me/aomona/auth/AuthBridge", "chooseFiles", "(Ljava/lang/Class;Z)Ljava/lang/String;");
        int drop = methodRef("me/aomona/auth/AuthBridge", "filesDrop", "(Ljava/lang/Object;Ljava/util/List;)V");
        int service = methodRef("me/aomona/auth/AuthBridge", "serviceConnection", "(Ljava/lang/Object;Ljava/net/URL;)Ljava/net/HttpURLConnection;");
        int codeName = text("Code");
        for (int i = 0; i < methods; i++) {
            int flags = in.readUnsignedShort(), name = in.readUnsignedShort(), desc = in.readUnsignedShort();
            String method = utf8.get(name), descriptor = utf8.get(desc);
            int kind = -1;
            if ("get".equals(method) && "(Ljava/net/URL;Ljava/lang/Class;)Ljava/lang/Object;".equals(descriptor)) kind = 0;
            if ("post".equals(method) && "(Ljava/net/URL;Ljava/lang/Class;)Ljava/lang/Object;".equals(descriptor)) kind = 1;
            if ("post".equals(method) && "(Ljava/net/URL;Ljava/lang/Object;Ljava/lang/Class;)Ljava/lang/Object;".equals(descriptor)) kind = 2;
            if (mode == 0 && "createUrlConnection".equals(method) && "(Ljava/net/URL;)Ljava/net/HttpURLConnection;".equals(descriptor)) kind = 9;
            if (crypt) {
                kind = -1;
                if ((flags & 0x0008) != 0 && "(Ljava/lang/String;)Ljava/security/PrivateKey;".equals(descriptor)) kind = 3;
                if ((flags & 0x0008) != 0 && "(Ljava/security/PrivateKey;)Ljava/lang/String;".equals(descriptor)) kind = 4;
            }
            if (mode == 2) kind = "(Ljava/net/URI;)V".equals(descriptor) && Set.of("openUri", "a", "method_673").contains(method) ? 5 : -1;
            if (mode == 3) kind = Set.of("connect", "d", "method_21054").contains(method)
                && (flags & 0x0408) == 0 && descriptor.startsWith("()L") ? 6 : -1;
            if (mode == 4) kind = "tinyfd_openFileDialog".equals(method)
                && "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;Lorg/lwjgl/PointerBuffer;Ljava/lang/CharSequence;Z)Ljava/lang/String;".equals(descriptor) ? 7 : -1;
            if (mode == 5) kind = (flags & 0x0008) == 0 && "(Ljava/util/List;)V".equals(descriptor)
                && Set.of("onFilesDrop", "a", "method_29638").contains(method) ? 8 : -1;
            String original = kind == 8 ? "onFilesDrop" : kind == 6 ? "connect" : kind == 3 ? "readPrivateKey" : kind == 4 ? "writePrivateKey" : method;
            ByteArrayOutputStream saved = new ByteArrayOutputStream(); DataOutputStream member = new DataOutputStream(saved);
            member.writeShort(flags); member.writeShort(kind < 0 ? name : text("mona$original$" + original)); member.writeShort(desc);
            copyAttributes(in, member);
            originals.add(saved.toByteArray());
            if (kind >= 0) {
                ByteArrayOutputStream bytes = new ByteArrayOutputStream(); DataOutputStream wrapper = new DataOutputStream(bytes);
                wrapper.writeShort(flags); wrapper.writeShort(name); wrapper.writeShort(desc); wrapper.writeShort(1);
                byte[] code = { 0x2a, 0x2b, (byte)(kind == 2 ? 0x2c : 0x01), (byte)(kind == 2 ? 0x2d : 0x2c),
                    (byte)(0x03 + kind), (byte)0xb8, (byte)(bridge >>> 8), (byte)bridge, (byte)0xb0 };
                if (crypt) {
                    int target = kind == 3 ? readKey : writeKey;
                    code = new byte[] { 0x13, (byte)(thisClass >>> 8), (byte)thisClass, 0x2a,
                        (byte)0xb8, (byte)(target >>> 8), (byte)target, (byte)0xb0 };
                }
                if (kind == 5) code = new byte[] { 0x13, (byte)(thisClass >>> 8), (byte)thisClass, (byte)((flags & 0x0008) != 0 ? 0x2a : 0x2b),
                    (byte)0xb8, (byte)(desktop >>> 8), (byte)desktop, (byte)0xb1 };
                if (kind == 6) code = new byte[] { 0x2a, (byte)0xb8, (byte)(realms >>> 8), (byte)realms,
                    (byte)0xc0, (byte)(thisClass >>> 8), (byte)thisClass, (byte)0xb0 };
                if (kind == 9) code = new byte[] { 0x2a, 0x2b, (byte)0xb8, (byte)(service >>> 8), (byte)service, (byte)0xb0 };
                if (kind == 7) code = new byte[] { 0x13, (byte)(thisClass >>> 8), (byte)thisClass, 0x15, 0x04,
                    (byte)0xb8, (byte)(choose >>> 8), (byte)choose, (byte)0xb0 };
                if (kind == 8) code = new byte[] { 0x2a, 0x2b, (byte)0xb8, (byte)(drop >>> 8), (byte)drop, (byte)0xb1 };
                wrapper.writeShort(codeName); wrapper.writeInt(12 + code.length);
                wrapper.writeShort(mode >= 1 ? 2 : 5); wrapper.writeShort(kind == 7 ? 5 : kind == 8 || kind == 5 ? 2 : kind == 6 || crypt ? 1 : kind == 2 ? 4 : 3);
                wrapper.writeInt(code.length); wrapper.write(code); wrapper.writeShort(0); wrapper.writeShort(0);
                wrappers.add(bytes.toByteArray());
            }
        }
        if (wrappers.size() != (mode >= 2 ? 1 : crypt ? 2 : 4)) throw new IOException("authlib signature mismatch");
        out.writeShort(methods + wrappers.size());
        for (byte[] method : originals) out.write(method);
        for (byte[] method : wrappers) out.write(method);
        copyAttributes(in, out);
        if (in.available() != 0) throw new IOException("unexpected class bytes");
        ByteArrayOutputStream result = new ByteArrayOutputStream(); DataOutputStream file = new DataOutputStream(result);
        file.writeInt(magic); file.writeShort(minor); file.writeShort(major); file.writeShort(count);
        file.write(constants.toByteArray()); file.write(body.toByteArray());
        return result.toByteArray();
    }
    private int text(String value) throws IOException { int index = count++; cp.writeByte(1); cp.writeUTF(value); return index; }
    private int clazz(String name) throws IOException { int text = text(name), index = count++; cp.writeByte(7); cp.writeShort(text); return index; }
    private int methodRef(String owner, String name, String descriptor) throws IOException {
        int cls = clazz(owner), method = text(name), desc = text(descriptor), pair = count++;
        cp.writeByte(12); cp.writeShort(method); cp.writeShort(desc);
        int index = count++; cp.writeByte(10); cp.writeShort(cls); cp.writeShort(pair); return index;
    }
    private static void copyMember(DataInputStream in, DataOutputStream out) throws IOException { out.write(in.readNBytes(6)); copyAttributes(in, out); }
    private static void copyAttributes(DataInputStream in, DataOutputStream out) throws IOException {
        int count = in.readUnsignedShort(); out.writeShort(count);
        for (int i = 0; i < count; i++) {
            out.writeShort(in.readUnsignedShort()); int size = in.readInt();
            if (size < 0 || size > 16 * 1024 * 1024) throw new IOException("oversized attribute");
            out.writeInt(size); byte[] bytes = in.readNBytes(size);
            if (bytes.length != size) throw new EOFException(); out.write(bytes);
        }
    }
}
