#include <jni.h>
#include <windows.h>
#include <stdio.h>
#include <string.h>

static HANDLE open_path(JNIEnv *env, jstring input, DWORD *error) {
    *error = 0;
    const jchar *path = (*env)->GetStringChars(env, input, NULL);
    if (!path) return INVALID_HANDLE_VALUE;
    jsize size = (*env)->GetStringLength(env, input);
    if (size > 32758) {
        (*env)->ReleaseStringChars(env, input, path);
        *error = ERROR_FILENAME_EXCED_RANGE;
        return INVALID_HANDLE_VALUE;
    }
    /* JNI strings need not be NUL terminated. Extended paths also support long names. */
    WCHAR *wide = (WCHAR *)HeapAlloc(GetProcessHeap(), 0, ((size_t)size + 9) * sizeof(WCHAR));
    if (!wide) {
        (*env)->ReleaseStringChars(env, input, path);
        jclass exception = (*env)->FindClass(env, "java/lang/OutOfMemoryError");
        if (exception) (*env)->ThrowNew(env, exception, "Windows path buffer allocation failed");
        return INVALID_HANDLE_VALUE;
    }
    int unc = size >= 2 && path[0] == '\\' && path[1] == '\\';
    const WCHAR *prefix = unc ? L"\\\\?\\UNC\\" : L"\\\\?\\";
    int prefix_size = unc ? 8 : 4, skip = unc ? 2 : 0;
    memcpy(wide, prefix, (size_t)prefix_size * sizeof(WCHAR));
    memcpy(wide + prefix_size, path + skip, (size_t)(size - skip) * sizeof(WCHAR));
    wide[size - skip + prefix_size] = 0;
    (*env)->ReleaseStringChars(env, input, path);
    HANDLE file = CreateFileW(wide, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL, OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS, NULL);
    *error = file == INVALID_HANDLE_VALUE ? GetLastError() : 0;
    HeapFree(GetProcessHeap(), 0, wide);
    return file;
}

static void path_error(JNIEnv *env, DWORD error) {
    jclass exception = (*env)->FindClass(env, error == ERROR_ACCESS_DENIED
        ? "java/nio/file/AccessDeniedException" : error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND
        ? "java/nio/file/NoSuchFileException" : "java/io/IOException");
    char message[96];
    snprintf(message, sizeof(message), "Windows path resolution failed: win32=%lu", (unsigned long)error);
    if (exception) (*env)->ThrowNew(env, exception, message);
}

JNIEXPORT jint JNICALL Java_me_aomona_sandbox_NativePaths_maxName(JNIEnv *env, jclass type, jstring input) {
    (void)type;
    DWORD error = 0, maximum = 0;
    HANDLE file = open_path(env, input, &error);
    if ((*env)->ExceptionCheck(env)) return 0;
    if (!error) {
        /* Query the accessible directory's volume through its handle, not C:\. */
        if (!GetVolumeInformationByHandleW(file, NULL, 0, NULL, &maximum, NULL, NULL, 0)) error = GetLastError();
        CloseHandle(file);
    }
    if (error) path_error(env, error);
    return (jint)maximum;
}

JNIEXPORT jstring JNICALL Java_me_aomona_sandbox_NativePaths_finalPath(JNIEnv *env, jclass type, jstring input) {
    (void)type;
    DWORD error = 0;
    HANDLE file = open_path(env, input, &error);
    if ((*env)->ExceptionCheck(env)) return NULL;
    WCHAR *result = NULL;
    DWORD length = 0;
    if (!error) {
        /* NT names avoid the mount-manager lookup required by VOLUME_NAME_DOS. */
        length = GetFinalPathNameByHandleW(file, NULL, 0, VOLUME_NAME_NT | FILE_NAME_NORMALIZED);
        if (!length) error = GetLastError();
        else {
            result = (WCHAR *)HeapAlloc(GetProcessHeap(), 0, ((size_t)length + 1) * sizeof(WCHAR));
            if (!result) error = ERROR_NOT_ENOUGH_MEMORY;
            else {
                DWORD actual = GetFinalPathNameByHandleW(file, result, length + 1, VOLUME_NAME_NT | FILE_NAME_NORMALIZED);
                if (!actual || actual > length) error = actual ? ERROR_INSUFFICIENT_BUFFER : GetLastError();
                else length = actual;
            }
        }
        CloseHandle(file);
    }
    jstring value = NULL;
    if (error) {
        path_error(env, error);
    } else value = (*env)->NewString(env, (const jchar *)result, (jsize)length);
    if (result) HeapFree(GetProcessHeap(), 0, result);
    return value;
}
