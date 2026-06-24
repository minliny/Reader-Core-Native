#include "reader_core.h"

#include <jni.h>

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <deque>
#include <mutex>
#include <string>

namespace {

struct RuntimeState {
  std::mutex mutex;
  std::condition_variable cv;
  std::deque<std::string> events;
  rc_runtime_t *runtime = nullptr;
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

}  // namespace

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
Java_com_reader_core_NativeCoreBridge_nativeLastError(JNIEnv *env,
                                                      jclass /*clazz*/) {
  char message[1024];
  message[0] = '\0';
  int32_t code = rc_last_error(message, sizeof(message));
  if (code == 0 || message[0] == '\0') {
    return env->NewStringUTF("");
  }
  return env->NewStringUTF(message);
}
