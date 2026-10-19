package me.aomona.sandbox;

import java.io.*;
import java.util.*;

/** Adapt two Windows filesystem operations without changing either class schema. */
final class PathAdapter {
    static byte[] transform(byte[] source, boolean realPath) throws IOException {
        DataInputStream in = new DataInputStream(new ByteArrayInputStream(source));
        ByteArrayOutputStream result = new ByteArrayOutputStream();
        DataOutputStream out = new DataOutputStream(result);
        out.write(in.readNBytes(8));
        int count = in.readUnsignedShort();
        out.writeShort(count + 6);
        Map<Integer, String> names = new HashMap<>();
        for (int i = 1; i < count; i++) {
            int tag = in.readUnsignedByte(); out.writeByte(tag);
            switch (tag) {
                case 1: String name = in.readUTF(); names.put(i, name); out.writeUTF(name); break;
                case 3: case 4: case 9: case 10: case 11: case 12: case 17: case 18: out.write(in.readNBytes(4)); break;
                case 5: case 6: out.write(in.readNBytes(8)); i++; break;
                case 7: case 8: case 16: case 19: case 20: out.write(in.readNBytes(2)); break;
                case 15: out.write(in.readNBytes(3)); break;
                default: throw new IOException("Unsupported class constant");
            }
        }
        out.writeByte(1); out.writeUTF("me/aomona/sandbox/NativePaths");
        out.writeByte(7); out.writeShort(count);
        out.writeByte(1); out.writeUTF(realPath ? "realPath" : "nameMax");
        out.writeByte(1); out.writeUTF(realPath ? "(Ljava/lang/Object;)Ljava/lang/String;" : "(Ljava/lang/String;)I");
        out.writeByte(12); out.writeShort(count + 2); out.writeShort(count + 3);
        out.writeByte(10); out.writeShort(count + 1); out.writeShort(count + 4);
        out.write(in.readNBytes(6));
        int interfaces = in.readUnsignedShort(); out.writeShort(interfaces); out.write(in.readNBytes(2 * interfaces));
        int fields = in.readUnsignedShort(); out.writeShort(fields);
        for (int i = 0; i < fields; i++) { out.write(in.readNBytes(6)); attributes(in, out); }
        int methods = in.readUnsignedShort(); out.writeShort(methods);
        int adapted = 0;
        for (int i = 0; i < methods; i++) {
            int flags = in.readUnsignedShort(), name = in.readUnsignedShort(), descriptor = in.readUnsignedShort();
            out.writeShort(flags); out.writeShort(name); out.writeShort(descriptor);
            boolean target = (realPath ? "getRealPath" : "getNameMax").equals(names.get(name))
                && (realPath ? "(Lsun/nio/fs/WindowsPath;Z)Ljava/lang/String;" : "(Ljava/lang/String;)I").equals(names.get(descriptor));
            int attributes = in.readUnsignedShort(); out.writeShort(attributes);
            for (int j = 0; j < attributes; j++) {
                int type = in.readUnsignedShort(), length = in.readInt();
                if (length < 0 || length > source.length) throw new IOException("Invalid attribute length");
                byte[] bytes = in.readNBytes(length);
                if (target && "Code".equals(names.get(type))) {
                    bytes = realPath ? code(bytes, count + 5, names) : nameMax(count + 5); adapted++;
                }
                out.writeShort(type); out.writeInt(bytes.length); out.write(bytes);
            }
        }
        attributes(in, out);
        if (adapted != 1 || in.available() != 0) throw new IOException("Windows path method mismatch");
        return result.toByteArray();
    }
    private static byte[] nameMax(int method) throws IOException {
        ByteArrayOutputStream result = new ByteArrayOutputStream(); DataOutputStream out = new DataOutputStream(result);
        out.writeShort(1); out.writeShort(2); out.writeInt(5);
        out.write(new byte[] { 0x2b, (byte)0xb8, (byte)(method >>> 8), (byte)method, (byte)0xac });
        out.writeShort(0); out.writeShort(0);
        return result.toByteArray();
    }
    private static byte[] code(byte[] source, int method, Map<Integer, String> names) throws IOException {
        DataInputStream in = new DataInputStream(new ByteArrayInputStream(source));
        ByteArrayOutputStream result = new ByteArrayOutputStream(); DataOutputStream out = new DataOutputStream(result);
        out.writeShort(Math.max(1, in.readUnsignedShort())); out.writeShort(in.readUnsignedShort());
        int length = in.readInt(); out.writeInt(length + 12);
        // Padding; iload_1; ifeq original; aload_0; invokestatic realPath; areturn.
        // A multiple of four preserves all original tableswitch/lookupswitch alignment.
        out.write(new byte[] { 0, 0, 0, 0x1b, (byte)0x99, 0, 8, 0x2a, (byte)0xb8,
            (byte)(method >>> 8), (byte)method, (byte)0xb0 });
        out.write(in.readNBytes(length));
        int exceptions = in.readUnsignedShort(); out.writeShort(exceptions);
        for (int i = 0; i < exceptions; i++) {
            for (int j = 0; j < 3; j++) out.writeShort(in.readUnsignedShort() + 12);
            out.writeShort(in.readUnsignedShort());
        }
        int attributes = in.readUnsignedShort(), stackName = -1; byte[] frames = null;
        for (int i = 0; i < attributes; i++) {
            int name = in.readUnsignedShort(), size = in.readInt(); byte[] value = in.readNBytes(size);
            if ("StackMapTable".equals(names.get(name))) { stackName = name; frames = value; }
        }
        if (frames == null) throw new IOException("Windows path stack map missing");
        DataInputStream frame = new DataInputStream(new ByteArrayInputStream(frames));
        int entries = frame.readUnsignedShort();
        if (entries == 0) throw new IOException("Windows path frames missing");
        ByteArrayOutputStream map = new ByteArrayOutputStream(); DataOutputStream stack = new DataOutputStream(map);
        stack.writeShort(entries + 1); stack.writeByte(12); // original method entry at offset 12
        for (int i = 0; i < entries; i++) {
            int tag = frame.readUnsignedByte();
            if (tag >= 128 && tag < 247) throw new IOException("Invalid Windows path frame");
            int delta = tag < 64 ? tag : tag < 128 ? tag - 64 : frame.readUnsignedShort();
            if (i == 0) {
                if (delta == 0) throw new IOException("Unsupported initial Windows path frame");
                delta--;
            }
            if (tag < 64) stack.writeByte(delta);
            else if (tag < 128) stack.writeByte(64 + delta);
            else { stack.writeByte(tag); stack.writeShort(delta); }
            if ((tag >= 64 && tag < 128) || tag == 247) verification(frame, stack);
            else if (tag >= 252 && tag <= 254) {
                for (int j = 0; j < tag - 251; j++) verification(frame, stack);
            } else if (tag == 255) {
                for (int group = 0; group < 2; group++) {
                    int types = frame.readUnsignedShort(); stack.writeShort(types);
                    for (int j = 0; j < types; j++) verification(frame, stack);
                }
            }
        }
        if (frame.available() != 0) throw new IOException("Invalid Windows path stack map");
        out.writeShort(1); out.writeShort(stackName); out.writeInt(map.size()); out.write(map.toByteArray());
        return result.toByteArray();
    }
    private static void verification(DataInputStream in, DataOutputStream out) throws IOException {
        int tag = in.readUnsignedByte();
        if (tag > 8) throw new IOException("Invalid verification type");
        out.writeByte(tag);
        if (tag == 7 || tag == 8) out.writeShort(in.readUnsignedShort() + (tag == 8 ? 12 : 0));
    }
    private static void attributes(DataInputStream in, DataOutputStream out) throws IOException {
        int count = in.readUnsignedShort(); out.writeShort(count);
        for (int i = 0; i < count; i++) {
            out.writeShort(in.readUnsignedShort()); int length = in.readInt();
            if (length < 0 || length > 16 * 1024 * 1024) throw new IOException("Invalid attribute length");
            out.writeInt(length); out.write(in.readNBytes(length));
        }
    }
}
