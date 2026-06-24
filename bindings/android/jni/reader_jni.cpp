#include "reader_core.h"

#include <jni.h>

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <string>

namespace {

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

void ThrowRuntimeException(JNIEnv *env, const char *message) {
  jclass exception_class = env->FindClass("java/lang/RuntimeException");
  if (exception_class != nullptr) {
    env->ThrowNew(exception_class, message);
  }
}

}  // namespace

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
