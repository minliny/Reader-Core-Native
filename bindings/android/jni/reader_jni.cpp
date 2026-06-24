// reader_jni.cpp — Android JNI bridge for Reader-Core (ABI v1).
//
// Exposes a persistent-handle lifecycle + bidirectional command/event bridge
// to Java/Kotlin:
//
//   com.reader.core.NativeCoreBridge:
//     @JvmStatic external fun abiVersion(): Int
//     @JvmStatic external fun pingSmoke(): String
//     @JvmStatic external fun runtimeCreate(configJson: String,
//                                           listener: ReaderEventListener): Long
//     @JvmStatic external fun runtimeSend(handle: Long, commandJson: String): Int
//     @JvmStatic external fun runtimeCancel(handle: Long, requestId: Long): Int
//     @JvmStatic external fun runtimeDestroy(handle: Long)
//
// Events (result / error / host.request) are delivered on a Core-owned
// background thread. The bridge attaches that thread to the JVM and invokes
// the Java listener's onEvent(String). host.complete / host.error answers
// are sent back by Java via runtimeSend — no ABI change required.

#include "reader_core.h"

#include <jni.h>

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <memory>
#include <mutex>
#include <new>
#include <string>

namespace {

// ---------------------------------------------------------------------------
// JVM + listener class caching
// ---------------------------------------------------------------------------

JavaVM *g_vm = nullptr;

std::once_flag g_listener_init;
jclass g_listener_class = nullptr;       // global ref
jmethodID g_on_event_mid = nullptr;

constexpr const char *kListenerClass = "com/reader/core/ReaderEventListener";
constexpr const char *kOnEventSig = "(Ljava/lang/String;)V";

void EnsureListenerCache(JNIEnv *env) {
  std::call_once(g_listener_init, [env] {
    jclass local = env->FindClass(kListenerClass);
    if (local == nullptr) {
      env->ExceptionClear();
      return;
    }
    g_listener_class = static_cast<jclass>(env->NewGlobalRef(local));
    env->DeleteLocalRef(local);
    g_on_event_mid = env->GetMethodID(g_listener_class, "onEvent", kOnEventSig);
    if (g_on_event_mid == nullptr) {
      env->ExceptionClear();
    }
  });
}

void ThrowRuntimeException(JNIEnv *env, const char *message) {
  jclass cls = env->FindClass("java/lang/RuntimeException");
  if (cls != nullptr) {
    env->ThrowNew(cls, message);
    env->DeleteLocalRef(cls);
  }
}

// ---------------------------------------------------------------------------
// Persistent runtime handle
// ---------------------------------------------------------------------------

struct RuntimeHandle {
  rc_runtime_t *runtime = nullptr;
  jobject listener_ref = nullptr;  // global ref to ReaderEventListener
};

// Delivered on a Core-owned background thread. Attach if needed; do NOT
// detach — Core may reuse this thread for future callbacks.
void OnEvent(void *context, const uint8_t *json, size_t json_length) {
  auto *handle = static_cast<RuntimeHandle *>(context);
  if (handle == nullptr || handle->listener_ref == nullptr || g_vm == nullptr) {
    return;
  }

  JNIEnv *env = nullptr;
  if (g_vm->GetEnv(reinterpret_cast<void **>(&env), JNI_VERSION_1_6) != JNI_OK) {
    if (g_vm->AttachCurrentThread(&env, nullptr) != JNI_OK) {
      return;
    }
  }

  if (env != nullptr && g_on_event_mid != nullptr && g_listener_class != nullptr) {
    std::string copy(reinterpret_cast<const char *>(json), json_length);
    jstring jstr = env->NewStringUTF(copy.c_str());
    env->CallVoidMethod(handle->listener_ref, g_on_event_mid, jstr);
    if (env->ExceptionCheck()) {
      env->ExceptionClear();
    }
    env->DeleteLocalRef(jstr);
  }
  // Leave the thread attached; Core owns its lifetime and may reuse it for
  // the next callback. Detaching here would force a re-attach and risk
  // freeing JNI state Core still depends on.
}

// ---------------------------------------------------------------------------
// One-shot smoke (preserved from the original shim for back-compat)
// ---------------------------------------------------------------------------

struct CapturedEvent {
  std::mutex mutex;
  std::condition_variable cv;
  std::string json;
};

void CaptureEvent(void *context, const uint8_t *json, size_t json_length) {
  auto *captured = static_cast<CapturedEvent *>(context);
  {
    std::lock_guard<std::mutex> lock(captured->mutex);
    captured->json.assign(reinterpret_cast<const char *>(json), json_length);
  }
  captured->cv.notify_one();
}

}  // namespace

extern "C" JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *vm, void * /*reserved*/) {
  g_vm = vm;
  JNIEnv *env = nullptr;
  if (vm->GetEnv(reinterpret_cast<void **>(&env), JNI_VERSION_1_6) != JNI_OK) {
    return JNI_ERR;
  }
  EnsureListenerCache(env);
  return JNI_VERSION_1_6;
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_abiVersion(JNIEnv * /*env*/,
                                                 jclass /*clazz*/) {
  return static_cast<jint>(rc_abi_version());
}

extern "C" JNIEXPORT jstring JNICALL
Java_com_reader_core_NativeCoreBridge_pingSmoke(JNIEnv *env, jclass /*clazz*/) {
  CapturedEvent captured;
  rc_runtime_t *runtime = nullptr;
  const char *config = "{}";
  int32_t code = rc_runtime_create(reinterpret_cast<const uint8_t *>(config),
                                   std::strlen(config), CaptureEvent,
                                   &captured, &runtime);
  if (code != 0 || runtime == nullptr) {
    ThrowRuntimeException(env, "rc_runtime_create failed");
    return nullptr;
  }

  const char *command =
      "{\"protocolVersion\":1,\"requestId\":42,\"method\":\"runtime.ping\",\"params\":{}}";
  code = rc_runtime_send(runtime, reinterpret_cast<const uint8_t *>(command),
                         std::strlen(command));
  if (code != 0) {
    rc_runtime_destroy(runtime);
    ThrowRuntimeException(env, "rc_runtime_send failed");
    return nullptr;
  }

  {
    std::unique_lock<std::mutex> lock(captured.mutex);
    captured.cv.wait_for(lock, std::chrono::seconds(1),
                         [&captured] { return !captured.json.empty(); });
  }

  rc_runtime_destroy(runtime);

  if (captured.json.empty()) {
    ThrowRuntimeException(env, "no event captured");
    return nullptr;
  }

  return env->NewStringUTF(captured.json.c_str());
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeCreate(JNIEnv *env, jclass /*clazz*/,
                                                    jstring config_json,
                                                    jobject listener) {
  if (listener == nullptr) {
    ThrowRuntimeException(env, "listener is null");
    return 0;
  }
  EnsureListenerCache(env);
  if (g_listener_class == nullptr || g_on_event_mid == nullptr) {
    ThrowRuntimeException(env, "ReaderEventListener class not resolvable");
    return 0;
  }

  auto *handle = new (std::nothrow) RuntimeHandle();
  if (handle == nullptr) {
    ThrowRuntimeException(env, "out of memory");
    return 0;
  }
  handle->listener_ref = env->NewGlobalRef(listener);

  std::string config;
  if (config_json != nullptr) {
    const char *chars = env->GetStringUTFChars(config_json, nullptr);
    if (chars != nullptr) {
      config.assign(chars);
      env->ReleaseStringUTFChars(config_json, chars);
    }
  }
  if (config.empty()) {
    config = "{}";
  }

  int32_t code = rc_runtime_create(
      reinterpret_cast<const uint8_t *>(config.data()), config.size(), OnEvent,
      handle, &handle->runtime);
  if (code != 0 || handle->runtime == nullptr) {
    env->DeleteGlobalRef(handle->listener_ref);
    delete handle;
    ThrowRuntimeException(env, "rc_runtime_create failed");
    return 0;
  }

  return reinterpret_cast<jlong>(handle);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeSend(JNIEnv *env, jclass /*clazz*/,
                                                  jlong handle_ptr,
                                                  jstring command_json) {
  auto *handle = reinterpret_cast<RuntimeHandle *>(handle_ptr);
  if (handle == nullptr || handle->runtime == nullptr) {
    ThrowRuntimeException(env, "invalid runtime handle");
    return 1;
  }
  if (command_json == nullptr) {
    ThrowRuntimeException(env, "command_json is null");
    return 2;
  }

  const char *chars = env->GetStringUTFChars(command_json, nullptr);
  if (chars == nullptr) {
    ThrowRuntimeException(env, "GetStringUTFChars failed");
    return 3;
  }
  size_t len = std::strlen(chars);
  int32_t code =
      rc_runtime_send(handle->runtime, reinterpret_cast<const uint8_t *>(chars), len);
  env->ReleaseStringUTFChars(command_json, chars);
  return static_cast<jint>(code);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeCancel(JNIEnv *env, jclass /*clazz*/,
                                                    jlong handle_ptr,
                                                    jlong request_id) {
  auto *handle = reinterpret_cast<RuntimeHandle *>(handle_ptr);
  if (handle == nullptr || handle->runtime == nullptr) {
    ThrowRuntimeException(env, "invalid runtime handle");
    return 1;
  }
  int32_t code = rc_runtime_cancel(handle->runtime,
                                   static_cast<uint64_t>(request_id));
  return static_cast<jint>(code);
}

extern "C" JNIEXPORT void JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeDestroy(JNIEnv *env, jclass /*clazz*/,
                                                     jlong handle_ptr) {
  auto *handle = reinterpret_cast<RuntimeHandle *>(handle_ptr);
  if (handle == nullptr) {
    return;
  }
  if (handle->runtime != nullptr) {
    rc_runtime_destroy(handle->runtime);
    handle->runtime = nullptr;
  }
  if (handle->listener_ref != nullptr && env != nullptr) {
    env->DeleteGlobalRef(handle->listener_ref);
    handle->listener_ref = nullptr;
  }
  delete handle;
}
