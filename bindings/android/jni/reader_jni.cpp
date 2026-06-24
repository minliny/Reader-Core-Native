#include "reader_core.h"

#include <jni.h>

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <deque>
#include <mutex>
#include <new>
#include <string>

namespace {

JavaVM *g_vm = nullptr;

struct RuntimeState {
  std::mutex mutex;
  std::condition_variable cv;
  std::deque<std::string> events;
  rc_runtime_t *runtime = nullptr;
  bool destroyed = false;
};

struct DirectRuntimeState {
  std::mutex mutex;
  rc_runtime_t *runtime = nullptr;
  jobject listener = nullptr;
  bool destroyed = false;
};

void CaptureEvent(void *context, const uint8_t *json, size_t json_length) {
  auto *state = static_cast<RuntimeState *>(context);
  std::string event(reinterpret_cast<const char *>(json), json_length);
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    if (state->destroyed) {
      return;
    }
    state->events.push_back(std::move(event));
  }
  state->cv.notify_one();
}

void ThrowException(JNIEnv *env, const char *class_name, const char *message) {
  jclass exception_class = env->FindClass(class_name);
  if (exception_class != nullptr) {
  env->ThrowNew(exception_class, message);
  }
}

void ThrowRuntimeException(JNIEnv *env, const char *message) {
  ThrowException(env, "java/lang/RuntimeException", message);
}

void ThrowIllegalState(JNIEnv *env, const char *message) {
  ThrowException(env, "java/lang/IllegalStateException", message);
}

RuntimeState *StateFromHandle(JNIEnv *env, jlong handle) {
  if (handle == 0) {
    ThrowIllegalState(env, "ReaderCore runtime handle is closed");
    return nullptr;
  }
  auto *state = reinterpret_cast<RuntimeState *>(static_cast<intptr_t>(handle));
  std::lock_guard<std::mutex> lock(state->mutex);
  if (state->destroyed || state->runtime == nullptr) {
    ThrowIllegalState(env, "ReaderCore runtime handle is closed");
    return nullptr;
  }
  return state;
}

const uint8_t *BytesOrNull(jbyte *bytes) {
  return reinterpret_cast<const uint8_t *>(bytes);
}

void DispatchDirectEvent(void *context, const uint8_t *json,
                         size_t json_length) {
  auto *state = static_cast<DirectRuntimeState *>(context);
  if (state == nullptr || g_vm == nullptr) {
    return;
  }

  JNIEnv *env = nullptr;
  if (g_vm->GetEnv(reinterpret_cast<void **>(&env), JNI_VERSION_1_6) != JNI_OK) {
    if (g_vm->AttachCurrentThread(&env, nullptr) != JNI_OK) {
      return;
    }
  }
  if (env == nullptr) {
    return;
  }

  jobject listener = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    if (state->destroyed || state->listener == nullptr) {
      return;
    }
    listener = state->listener;
  }

  jclass listener_class = env->GetObjectClass(listener);
  if (listener_class == nullptr) {
    env->ExceptionClear();
    return;
  }
  jmethodID on_event =
      env->GetMethodID(listener_class, "onEvent", "(Ljava/lang/String;)V");
  env->DeleteLocalRef(listener_class);
  if (on_event == nullptr) {
    env->ExceptionClear();
    return;
  }

  std::string event(reinterpret_cast<const char *>(json), json_length);
  jstring event_json = env->NewStringUTF(event.c_str());
  if (event_json == nullptr) {
    env->ExceptionClear();
    return;
  }
  env->CallVoidMethod(listener, on_event, event_json);
  if (env->ExceptionCheck()) {
    env->ExceptionClear();
  }
  env->DeleteLocalRef(event_json);
}

}  // namespace

extern "C" JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *vm, void * /*reserved*/) {
  g_vm = vm;
  return JNI_VERSION_1_6;
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_nativeAbiVersion(JNIEnv * /*env*/,
                                                       jclass /*clazz*/) {
  return static_cast<jint>(rc_abi_version());
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_reader_core_NativeCoreBridge_nativeCreate(JNIEnv *env,
                                                   jclass /*clazz*/,
                                                   jbyteArray config_json) {
  auto *state = new RuntimeState();

  jbyte *config_bytes = nullptr;
  jsize config_length = 0;
  if (config_json != nullptr) {
    config_length = env->GetArrayLength(config_json);
    config_bytes = env->GetByteArrayElements(config_json, nullptr);
    if (config_bytes == nullptr) {
      delete state;
      return 0;
    }
  }

  int32_t code = rc_runtime_create(
      BytesOrNull(config_bytes), static_cast<size_t>(config_length),
      CaptureEvent, state, &state->runtime);

  if (config_bytes != nullptr) {
    env->ReleaseByteArrayElements(config_json, config_bytes, JNI_ABORT);
  }

  if (code != 0 || state->runtime == nullptr) {
    delete state;
    return 0;
  }

  return static_cast<jlong>(reinterpret_cast<intptr_t>(state));
}

extern "C" JNIEXPORT void JNICALL
Java_com_reader_core_NativeCoreBridge_nativeDestroy(JNIEnv *env,
                                                    jclass /*clazz*/,
                                                    jlong handle) {
  auto *state = StateFromHandle(env, handle);
  if (state == nullptr) {
    return;
  }

  rc_runtime_t *runtime = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    runtime = state->runtime;
    state->runtime = nullptr;
    state->destroyed = true;
    state->events.clear();
  }
  state->cv.notify_all();

  if (runtime != nullptr) {
    rc_runtime_destroy(runtime);
  }

  delete state;
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_nativeSend(JNIEnv *env, jclass /*clazz*/,
                                                 jlong handle,
                                                 jbyteArray command_json) {
  auto *state = StateFromHandle(env, handle);
  if (state == nullptr) {
    return -1;
  }

  jbyte *command_bytes = nullptr;
  jsize command_length = 0;
  if (command_json != nullptr) {
    command_length = env->GetArrayLength(command_json);
    command_bytes = env->GetByteArrayElements(command_json, nullptr);
    if (command_bytes == nullptr) {
      return -1;
    }
  }

  rc_runtime_t *runtime = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    runtime = state->runtime;
  }

  int32_t code = rc_runtime_send(
      runtime, BytesOrNull(command_bytes), static_cast<size_t>(command_length));

  if (command_bytes != nullptr) {
    env->ReleaseByteArrayElements(command_json, command_bytes, JNI_ABORT);
  }

  return static_cast<jint>(code);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_nativeCancel(JNIEnv *env,
                                                   jclass /*clazz*/,
                                                   jlong handle,
                                                   jlong request_id) {
  auto *state = StateFromHandle(env, handle);
  if (state == nullptr) {
    return -1;
  }

  rc_runtime_t *runtime = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    runtime = state->runtime;
  }

  return static_cast<jint>(
      rc_runtime_cancel(runtime, static_cast<uint64_t>(request_id)));
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_reader_core_NativeCoreBridge_nativePollEvent(JNIEnv *env,
                                                      jclass /*clazz*/,
                                                      jlong handle,
                                                      jlong timeout_millis) {
  auto *state = StateFromHandle(env, handle);
  if (state == nullptr) {
    return nullptr;
  }

  std::string event;
  {
    std::unique_lock<std::mutex> lock(state->mutex);
    auto has_event_or_closed = [state] {
      return state->destroyed || !state->events.empty();
    };

    if (timeout_millis <= 0) {
      if (!has_event_or_closed()) {
        return nullptr;
      }
    } else {
      state->cv.wait_for(lock, std::chrono::milliseconds(timeout_millis),
                         has_event_or_closed);
    }

    if (state->destroyed || state->events.empty()) {
      return nullptr;
    }

    event = std::move(state->events.front());
    state->events.pop_front();
  }

  jbyteArray result = env->NewByteArray(static_cast<jsize>(event.size()));
  if (result == nullptr) {
    return nullptr;
  }
  env->SetByteArrayRegion(result, 0, static_cast<jsize>(event.size()),
                          reinterpret_cast<const jbyte *>(event.data()));
  return result;
}

extern "C" JNIEXPORT jstring JNICALL
Java_com_reader_core_NativeCoreBridge_pingSmoke(JNIEnv *env,
                                                jclass /*clazz*/) {
  RuntimeState state;
  const char *config = "{}";
  int32_t code = rc_runtime_create(reinterpret_cast<const uint8_t *>(config),
                                   std::strlen(config), CaptureEvent, &state,
                                   &state.runtime);
  if (code != 0 || state.runtime == nullptr) {
    ThrowRuntimeException(env, "rc_runtime_create failed");
    return nullptr;
  }

  const char *command =
      "{\"protocolVersion\":1,\"requestId\":42,\"method\":\"runtime.ping\",\"params\":{}}";
  code = rc_runtime_send(state.runtime, reinterpret_cast<const uint8_t *>(command),
                         std::strlen(command));
  if (code != 0) {
    rc_runtime_destroy(state.runtime);
    state.runtime = nullptr;
    ThrowRuntimeException(env, "rc_runtime_send failed");
    return nullptr;
  }

  std::string event;
  {
    std::unique_lock<std::mutex> lock(state.mutex);
    state.cv.wait_for(lock, std::chrono::seconds(1),
                      [&state] { return !state.events.empty(); });
    if (!state.events.empty()) {
      event = std::move(state.events.front());
    }
  }

  rc_runtime_destroy(state.runtime);
  state.runtime = nullptr;

  if (event.empty()) {
    ThrowRuntimeException(env, "runtime.ping timed out");
    return nullptr;
  }
  return env->NewStringUTF(event.c_str());
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeCreate(JNIEnv *env,
                                                    jclass /*clazz*/,
                                                    jstring config_json,
                                                    jobject listener) {
  if (listener == nullptr) {
    ThrowRuntimeException(env, "listener is null");
    return 0;
  }

  auto *state = new (std::nothrow) DirectRuntimeState();
  if (state == nullptr) {
    ThrowRuntimeException(env, "out of memory");
    return 0;
  }
  state->listener = env->NewGlobalRef(listener);
  if (state->listener == nullptr) {
    delete state;
    ThrowRuntimeException(env, "listener global ref failed");
    return 0;
  }

  std::string config = "{}";
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
      reinterpret_cast<const uint8_t *>(config.data()), config.size(),
      DispatchDirectEvent, state, &state->runtime);
  if (code != 0 || state->runtime == nullptr) {
    env->DeleteGlobalRef(state->listener);
    delete state;
    ThrowRuntimeException(env, "rc_runtime_create failed");
    return 0;
  }

  return static_cast<jlong>(reinterpret_cast<intptr_t>(state));
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeSend(JNIEnv *env,
                                                  jclass /*clazz*/,
                                                  jlong handle,
                                                  jstring command_json) {
  auto *state = reinterpret_cast<DirectRuntimeState *>(static_cast<intptr_t>(handle));
  if (state == nullptr || command_json == nullptr) {
    ThrowRuntimeException(env, "invalid runtime send arguments");
    return -1;
  }

  rc_runtime_t *runtime = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    if (state->destroyed || state->runtime == nullptr) {
      ThrowRuntimeException(env, "ReaderCore runtime handle is closed");
      return -1;
    }
    runtime = state->runtime;
  }

  const char *chars = env->GetStringUTFChars(command_json, nullptr);
  if (chars == nullptr) {
    return -1;
  }
  int32_t code = rc_runtime_send(
      runtime, reinterpret_cast<const uint8_t *>(chars), std::strlen(chars));
  env->ReleaseStringUTFChars(command_json, chars);
  return static_cast<jint>(code);
}

extern "C" JNIEXPORT jint JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeCancel(JNIEnv *env,
                                                    jclass /*clazz*/,
                                                    jlong handle,
                                                    jlong request_id) {
  auto *state = reinterpret_cast<DirectRuntimeState *>(static_cast<intptr_t>(handle));
  if (state == nullptr) {
    ThrowRuntimeException(env, "invalid runtime handle");
    return -1;
  }

  rc_runtime_t *runtime = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    if (state->destroyed || state->runtime == nullptr) {
      ThrowRuntimeException(env, "ReaderCore runtime handle is closed");
      return -1;
    }
    runtime = state->runtime;
  }

  return static_cast<jint>(
      rc_runtime_cancel(runtime, static_cast<uint64_t>(request_id)));
}

extern "C" JNIEXPORT void JNICALL
Java_com_reader_core_NativeCoreBridge_runtimeDestroy(JNIEnv *env,
                                                     jclass /*clazz*/,
                                                     jlong handle) {
  auto *state = reinterpret_cast<DirectRuntimeState *>(static_cast<intptr_t>(handle));
  if (state == nullptr) {
    return;
  }

  rc_runtime_t *runtime = nullptr;
  jobject listener = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    runtime = state->runtime;
    state->runtime = nullptr;
    listener = state->listener;
    state->listener = nullptr;
    state->destroyed = true;
  }

  if (runtime != nullptr) {
    rc_runtime_destroy(runtime);
  }
  if (listener != nullptr) {
    env->DeleteGlobalRef(listener);
  }
  delete state;
}
