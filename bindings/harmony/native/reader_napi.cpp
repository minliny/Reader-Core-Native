#include "reader_core.h"

#include <node_api.h>

#include <condition_variable>
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

napi_value ThrowError(napi_env env, const char *message) {
  napi_throw_error(env, nullptr, message);
  return nullptr;
}

napi_value AbiVersion(napi_env env, napi_callback_info /*info*/) {
  napi_value result = nullptr;
  napi_status status = napi_create_uint32(env, rc_abi_version(), &result);
  if (status != napi_ok) {
    return ThrowError(env, "failed to create ABI version value");
  }
  return result;
}

napi_value PingSmoke(napi_env env, napi_callback_info /*info*/) {
  CapturedEvent captured;
  rc_runtime_t *runtime = nullptr;
  const char *config = "{}";
  int32_t code = rc_runtime_create(reinterpret_cast<const uint8_t *>(config),
                                   std::strlen(config), CaptureEvent,
                                   &captured, &runtime);
  if (code != 0 || runtime == nullptr) {
    return ThrowError(env, "rc_runtime_create failed");
  }

  const char *command =
      "{\"protocolVersion\":1,\"requestId\":42,\"method\":\"core.ping\",\"params\":{}}";
  code = rc_runtime_send(runtime, reinterpret_cast<const uint8_t *>(command),
                         std::strlen(command));
  if (code != 0) {
    rc_runtime_destroy(runtime);
    return ThrowError(env, "rc_runtime_send failed");
  }

  {
    std::unique_lock<std::mutex> lock(captured.mutex);
    captured.cv.wait_for(lock, std::chrono::seconds(1),
                         [&captured] { return !captured.json.empty(); });
  }

  rc_runtime_destroy(runtime);

  if (captured.json.empty()) {
    return ThrowError(env, "no event captured");
  }

  napi_value result = nullptr;
  napi_status status = napi_create_string_utf8(env, captured.json.c_str(),
                                               captured.json.size(), &result);
  if (status != napi_ok) {
    return ThrowError(env, "failed to create ping result value");
  }
  return result;
}

napi_value Init(napi_env env, napi_value exports) {
  napi_property_descriptor properties[] = {
      {"abiVersion", nullptr, AbiVersion, nullptr, nullptr, nullptr, napi_default,
       nullptr},
      {"pingSmoke", nullptr, PingSmoke, nullptr, nullptr, nullptr, napi_default,
       nullptr},
  };
  napi_status status = napi_define_properties(
      env, exports, sizeof(properties) / sizeof(properties[0]), properties);
  if (status != napi_ok) {
    return ThrowError(env, "failed to define reader_core_napi exports");
  }
  return exports;
}

}  // namespace

NAPI_MODULE(reader_core_napi, Init)
