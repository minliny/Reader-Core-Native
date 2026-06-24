#include "reader_core.h"

#include <node_api.h>

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <deque>
#include <mutex>
#include <string>
#include <utility>

namespace {

constexpr uint64_t kDefaultHostResponseRequestId = 9000000000000ULL;

struct RuntimeState {
  std::mutex mutex;
  std::condition_variable cv;
  std::deque<std::string> events;
  rc_runtime_t *runtime = nullptr;
  uint64_t next_host_response_request_id = kDefaultHostResponseRequestId;
  bool destroyed = false;
};

void RuntimeEventCallback(void *context, const uint8_t *json,
                          size_t json_length) {
  auto *state = static_cast<RuntimeState *>(context);
  if (state == nullptr || json == nullptr) {
    return;
  }

  {
    std::lock_guard<std::mutex> lock(state->mutex);
    if (state->destroyed) {
      return;
    }
    state->events.emplace_back(reinterpret_cast<const char *>(json),
                               json_length);
  }
  state->cv.notify_one();
}

napi_value ThrowError(napi_env env, const char *message) {
  napi_throw_error(env, nullptr, message);
  return nullptr;
}

napi_value ThrowError(napi_env env, const std::string &message) {
  return ThrowError(env, message.c_str());
}

bool IsNullish(napi_env env, napi_value value) {
  napi_valuetype type = napi_undefined;
  return napi_typeof(env, value, &type) == napi_ok &&
         (type == napi_undefined || type == napi_null);
}

napi_value Undefined(napi_env env) {
  napi_value result = nullptr;
  napi_get_undefined(env, &result);
  return result;
}

napi_value Null(napi_env env) {
  napi_value result = nullptr;
  napi_get_null(env, &result);
  return result;
}

bool GetStringValue(napi_env env, napi_value value, std::string *out) {
  size_t length = 0;
  napi_status status =
      napi_get_value_string_utf8(env, value, nullptr, 0, &length);
  if (status != napi_ok) {
    ThrowError(env, "expected string");
    return false;
  }

  std::string buffer(length + 1, '\0');
  size_t copied = 0;
  status =
      napi_get_value_string_utf8(env, value, buffer.data(), buffer.size(), &copied);
  if (status != napi_ok) {
    ThrowError(env, "failed to read string");
    return false;
  }
  buffer.resize(copied);
  *out = std::move(buffer);
  return true;
}

bool JsonStringify(napi_env env, napi_value value, std::string *out) {
  napi_value global = nullptr;
  napi_value json = nullptr;
  napi_value stringify = nullptr;
  napi_value result = nullptr;

  if (napi_get_global(env, &global) != napi_ok ||
      napi_get_named_property(env, global, "JSON", &json) != napi_ok ||
      napi_get_named_property(env, json, "stringify", &stringify) != napi_ok ||
      napi_call_function(env, json, stringify, 1, &value, &result) != napi_ok ||
      IsNullish(env, result)) {
    ThrowError(env, "failed to stringify JSON argument");
    return false;
  }

  return GetStringValue(env, result, out);
}

bool GetJsonStringValue(napi_env env, napi_value value, std::string *out) {
  napi_valuetype type = napi_undefined;
  if (napi_typeof(env, value, &type) != napi_ok) {
    ThrowError(env, "failed to inspect JSON argument");
    return false;
  }

  if (type == napi_string) {
    return GetStringValue(env, value, out);
  }
  if (type == napi_object) {
    return JsonStringify(env, value, out);
  }

  ThrowError(env, "expected JSON string or object");
  return false;
}

bool GetOptionalJsonString(napi_env env, size_t argc, napi_value *args,
                           size_t index, const char *fallback,
                           std::string *out) {
  if (index >= argc || IsNullish(env, args[index])) {
    *out = fallback;
    return true;
  }
  return GetJsonStringValue(env, args[index], out);
}

bool GetInt64Value(napi_env env, napi_value value, int64_t *out) {
  napi_status status = napi_get_value_int64(env, value, out);
  if (status != napi_ok) {
    ThrowError(env, "expected integer");
    return false;
  }
  return true;
}

bool GetOptionalInt64(napi_env env, size_t argc, napi_value *args, size_t index,
                      int64_t fallback, int64_t *out) {
  if (index >= argc || IsNullish(env, args[index])) {
    *out = fallback;
    return true;
  }
  return GetInt64Value(env, args[index], out);
}

RuntimeState *GetRuntimeState(napi_env env, napi_value value,
                              bool require_active) {
  RuntimeState *state = nullptr;
  napi_status status =
      napi_get_value_external(env, value, reinterpret_cast<void **>(&state));
  if (status != napi_ok || state == nullptr) {
    ThrowError(env, "expected runtime handle");
    return nullptr;
  }

  if (require_active) {
    std::lock_guard<std::mutex> lock(state->mutex);
    if (state->runtime == nullptr || state->destroyed) {
      ThrowError(env, "runtime already released");
      return nullptr;
    }
  }

  return state;
}

void DestroyRuntimeState(RuntimeState *state) {
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

  if (runtime != nullptr) {
    rc_runtime_destroy(runtime);
  }
  state->cv.notify_all();
}

void RuntimeFinalizer(napi_env /*env*/, void *finalize_data,
                      void * /*finalize_hint*/) {
  auto *state = static_cast<RuntimeState *>(finalize_data);
  DestroyRuntimeState(state);
  delete state;
}

int32_t SendCommandToCore(RuntimeState *state, const std::string &command) {
  rc_runtime_t *runtime = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    runtime = state->runtime;
  }
  if (runtime == nullptr) {
    return -1;
  }
  return rc_runtime_send(runtime, reinterpret_cast<const uint8_t *>(command.data()),
                         command.size());
}

bool WaitForEvent(RuntimeState *state, int64_t timeout_ms, std::string *event) {
  std::unique_lock<std::mutex> lock(state->mutex);
  if (state->events.empty() && timeout_ms > 0 && !state->destroyed) {
    state->cv.wait_for(lock, std::chrono::milliseconds(timeout_ms), [state] {
      return !state->events.empty() || state->destroyed;
    });
  }

  if (state->events.empty()) {
    return false;
  }

  *event = std::move(state->events.front());
  state->events.pop_front();
  return true;
}

std::string TrimForJsonObjectCheck(const std::string &json) {
  size_t begin = 0;
  while (begin < json.size() &&
         (json[begin] == ' ' || json[begin] == '\n' || json[begin] == '\r' ||
          json[begin] == '\t')) {
    ++begin;
  }

  size_t end = json.size();
  while (end > begin &&
         (json[end - 1] == ' ' || json[end - 1] == '\n' ||
          json[end - 1] == '\r' || json[end - 1] == '\t')) {
    --end;
  }

  return json.substr(begin, end - begin);
}

bool LooksLikeJsonObject(const std::string &json) {
  std::string trimmed = TrimForJsonObjectCheck(json);
  return trimmed.size() >= 2 && trimmed.front() == '{' &&
         trimmed.back() == '}';
}

std::string BuildHostCompleteCommand(uint64_t request_id, uint64_t operation_id,
                                     const std::string &result_json) {
  return "{\"protocolVersion\":1,\"requestId\":" + std::to_string(request_id) +
         ",\"method\":\"host.complete\",\"params\":{\"operationId\":" +
         std::to_string(operation_id) + ",\"result\":" + result_json + "}}";
}

std::string BuildHostErrorCommand(uint64_t request_id, uint64_t operation_id,
                                  const std::string &error_json) {
  return "{\"protocolVersion\":1,\"requestId\":" + std::to_string(request_id) +
         ",\"method\":\"host.error\",\"params\":{\"operationId\":" +
         std::to_string(operation_id) + ",\"error\":" + error_json + "}}";
}

bool ExtractUnsignedField(const std::string &json, const char *field,
                          uint64_t *out) {
  const std::string key = std::string("\"") + field + "\"";
  size_t pos = json.find(key);
  if (pos == std::string::npos) {
    return false;
  }
  pos = json.find(':', pos + key.size());
  if (pos == std::string::npos) {
    return false;
  }
  ++pos;
  while (pos < json.size() &&
         (json[pos] == ' ' || json[pos] == '\n' || json[pos] == '\r' ||
          json[pos] == '\t')) {
    ++pos;
  }
  if (pos >= json.size() || json[pos] < '0' || json[pos] > '9') {
    return false;
  }

  uint64_t value = 0;
  while (pos < json.size() && json[pos] >= '0' && json[pos] <= '9') {
    value = value * 10 + static_cast<uint64_t>(json[pos] - '0');
    ++pos;
  }
  *out = value;
  return true;
}

napi_value StringResult(napi_env env, const std::string &value,
                        const char *error_message) {
  napi_value result = nullptr;
  napi_status status =
      napi_create_string_utf8(env, value.c_str(), value.size(), &result);
  if (status != napi_ok) {
    return ThrowError(env, error_message);
  }
  return result;
}

napi_value AbiVersion(napi_env env, napi_callback_info /*info*/) {
  napi_value result = nullptr;
  napi_status status = napi_create_uint32(env, rc_abi_version(), &result);
  if (status != napi_ok) {
    return ThrowError(env, "failed to create ABI version value");
  }
  return result;
}

napi_value CreateRuntime(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value args[1] = {nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok) {
    return ThrowError(env, "failed to read createRuntime arguments");
  }

  std::string config;
  if (!GetOptionalJsonString(env, argc, args, 0, "{}", &config)) {
    return nullptr;
  }

  auto *state = new RuntimeState();
  int32_t code = rc_runtime_create(
      reinterpret_cast<const uint8_t *>(config.data()), config.size(),
      RuntimeEventCallback, state, &state->runtime);
  if (code != 0 || state->runtime == nullptr) {
    delete state;
    return ThrowError(env, "rc_runtime_create failed");
  }

  napi_value external = nullptr;
  napi_status status =
      napi_create_external(env, state, RuntimeFinalizer, nullptr, &external);
  if (status != napi_ok) {
    DestroyRuntimeState(state);
    delete state;
    return ThrowError(env, "failed to create runtime handle");
  }
  return external;
}

napi_value ReleaseRuntime(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value args[1] = {nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok ||
      argc < 1) {
    return ThrowError(env, "releaseRuntime requires runtime handle");
  }

  RuntimeState *state = GetRuntimeState(env, args[0], false);
  if (state == nullptr) {
    return nullptr;
  }

  DestroyRuntimeState(state);
  return Undefined(env);
}

napi_value SendCommand(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value args[2] = {nullptr, nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok ||
      argc < 2) {
    return ThrowError(env, "sendCommand requires runtime handle and command");
  }

  RuntimeState *state = GetRuntimeState(env, args[0], true);
  if (state == nullptr) {
    return nullptr;
  }

  std::string command;
  if (!GetJsonStringValue(env, args[1], &command)) {
    return nullptr;
  }

  int32_t code = SendCommandToCore(state, command);
  if (code != 0) {
    return ThrowError(env, "rc_runtime_send failed");
  }
  return Undefined(env);
}

napi_value CancelRequest(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value args[2] = {nullptr, nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok ||
      argc < 2) {
    return ThrowError(env, "cancelRequest requires runtime handle and requestId");
  }

  RuntimeState *state = GetRuntimeState(env, args[0], true);
  if (state == nullptr) {
    return nullptr;
  }

  int64_t request_id_signed = 0;
  if (!GetInt64Value(env, args[1], &request_id_signed)) {
    return nullptr;
  }
  if (request_id_signed < 0) {
    return ThrowError(env, "requestId must be a non-negative integer");
  }

  rc_runtime_t *runtime = nullptr;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    runtime = state->runtime;
  }
  if (runtime == nullptr) {
    return ThrowError(env, "runtime already released");
  }

  int32_t code = rc_runtime_cancel(runtime, static_cast<uint64_t>(request_id_signed));
  if (code != 0) {
    return ThrowError(env, "rc_runtime_cancel failed");
  }
  return Undefined(env);
}

napi_value ReadEvent(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value args[2] = {nullptr, nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok ||
      argc < 1) {
    return ThrowError(env, "readEvent requires runtime handle");
  }

  RuntimeState *state = GetRuntimeState(env, args[0], false);
  if (state == nullptr) {
    return nullptr;
  }

  int64_t timeout_ms = 0;
  if (!GetOptionalInt64(env, argc, args, 1, 0, &timeout_ms)) {
    return nullptr;
  }
  if (timeout_ms < 0) {
    timeout_ms = 0;
  }

  std::string event;
  if (!WaitForEvent(state, timeout_ms, &event)) {
    return Null(env);
  }
  return StringResult(env, event, "failed to create event string");
}

napi_value PendingEventCount(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value args[1] = {nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok ||
      argc < 1) {
    return ThrowError(env, "pendingEventCount requires runtime handle");
  }

  RuntimeState *state = GetRuntimeState(env, args[0], false);
  if (state == nullptr) {
    return nullptr;
  }

  uint32_t count = 0;
  {
    std::lock_guard<std::mutex> lock(state->mutex);
    count = static_cast<uint32_t>(state->events.size());
  }

  napi_value result = nullptr;
  if (napi_create_uint32(env, count, &result) != napi_ok) {
    return ThrowError(env, "failed to create pending event count");
  }
  return result;
}

napi_value CompleteHostRequest(napi_env env, napi_callback_info info) {
  size_t argc = 4;
  napi_value args[4] = {nullptr, nullptr, nullptr, nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok ||
      argc < 3) {
    return ThrowError(
        env,
        "completeHostRequest requires runtime handle, operationId, and result");
  }

  RuntimeState *state = GetRuntimeState(env, args[0], true);
  if (state == nullptr) {
    return nullptr;
  }

  int64_t operation_id_signed = 0;
  if (!GetInt64Value(env, args[1], &operation_id_signed)) {
    return nullptr;
  }
  if (operation_id_signed < 0) {
    return ThrowError(env, "operationId must be a non-negative integer");
  }

  std::string result_json;
  if (!GetJsonStringValue(env, args[2], &result_json)) {
    return nullptr;
  }
  if (!LooksLikeJsonObject(result_json)) {
    return ThrowError(env, "host.complete result must be a JSON object");
  }

  uint64_t request_id = 0;
  if (argc > 3 && !IsNullish(env, args[3])) {
    int64_t request_id_signed = 0;
    if (!GetInt64Value(env, args[3], &request_id_signed)) {
      return nullptr;
    }
    if (request_id_signed < 0) {
      return ThrowError(env, "requestId must be a non-negative integer");
    }
    request_id = static_cast<uint64_t>(request_id_signed);
  } else {
    std::lock_guard<std::mutex> lock(state->mutex);
    request_id = state->next_host_response_request_id++;
  }

  std::string command = BuildHostCompleteCommand(
      request_id, static_cast<uint64_t>(operation_id_signed), result_json);
  int32_t code = SendCommandToCore(state, command);
  if (code != 0) {
    return ThrowError(env, "rc_runtime_send host.complete failed");
  }
  return Undefined(env);
}

napi_value FailHostRequest(napi_env env, napi_callback_info info) {
  size_t argc = 4;
  napi_value args[4] = {nullptr, nullptr, nullptr, nullptr};
  if (napi_get_cb_info(env, info, &argc, args, nullptr, nullptr) != napi_ok ||
      argc < 3) {
    return ThrowError(
        env,
        "failHostRequest requires runtime handle, operationId, and error");
  }

  RuntimeState *state = GetRuntimeState(env, args[0], true);
  if (state == nullptr) {
    return nullptr;
  }

  int64_t operation_id_signed = 0;
  if (!GetInt64Value(env, args[1], &operation_id_signed)) {
    return nullptr;
  }
  if (operation_id_signed < 0) {
    return ThrowError(env, "operationId must be a non-negative integer");
  }

  std::string error_json;
  if (!GetJsonStringValue(env, args[2], &error_json)) {
    return nullptr;
  }
  if (!LooksLikeJsonObject(error_json)) {
    return ThrowError(env, "host.error error must be a JSON object");
  }

  uint64_t request_id = 0;
  if (argc > 3 && !IsNullish(env, args[3])) {
    int64_t request_id_signed = 0;
    if (!GetInt64Value(env, args[3], &request_id_signed)) {
      return nullptr;
    }
    if (request_id_signed < 0) {
      return ThrowError(env, "requestId must be a non-negative integer");
    }
    request_id = static_cast<uint64_t>(request_id_signed);
  } else {
    std::lock_guard<std::mutex> lock(state->mutex);
    request_id = state->next_host_response_request_id++;
  }

  std::string command = BuildHostErrorCommand(
      request_id, static_cast<uint64_t>(operation_id_signed), error_json);
  int32_t code = SendCommandToCore(state, command);
  if (code != 0) {
    return ThrowError(env, "rc_runtime_send host.error failed");
  }
  return Undefined(env);
}

napi_value PingSmoke(napi_env env, napi_callback_info /*info*/) {
  RuntimeState state;
  const char *config = "{}";
  int32_t code = rc_runtime_create(reinterpret_cast<const uint8_t *>(config),
                                   std::strlen(config), RuntimeEventCallback,
                                   &state, &state.runtime);
  if (code != 0 || state.runtime == nullptr) {
    return ThrowError(env, "rc_runtime_create failed");
  }

  const char *command =
      "{\"protocolVersion\":1,\"requestId\":42,\"method\":\"runtime.ping\",\"params\":{}}";
  code = rc_runtime_send(state.runtime, reinterpret_cast<const uint8_t *>(command),
                         std::strlen(command));
  if (code != 0) {
    DestroyRuntimeState(&state);
    return ThrowError(env, "rc_runtime_send failed");
  }

  std::string event;
  bool captured = WaitForEvent(&state, 1000, &event);
  DestroyRuntimeState(&state);

  if (!captured) {
    return ThrowError(env, "no event captured");
  }

  return StringResult(env, event, "failed to create ping result value");
}

napi_value HostSmoke(napi_env env, napi_callback_info /*info*/) {
  RuntimeState state;
  const char *config = "{}";
  int32_t code = rc_runtime_create(reinterpret_cast<const uint8_t *>(config),
                                   std::strlen(config), RuntimeEventCallback,
                                   &state, &state.runtime);
  if (code != 0 || state.runtime == nullptr) {
    return ThrowError(env, "rc_runtime_create failed");
  }

  const char *host_smoke =
      "{\"protocolVersion\":1,\"requestId\":10,\"method\":\"runtime.hostSmoke\","
      "\"params\":{\"capability\":\"host.smoke.echo\",\"params\":{\"status\":\"ok\"}}}";
  code = rc_runtime_send(state.runtime,
                         reinterpret_cast<const uint8_t *>(host_smoke),
                         std::strlen(host_smoke));
  if (code != 0) {
    DestroyRuntimeState(&state);
    return ThrowError(env, "rc_runtime_send runtime.hostSmoke failed");
  }

  std::string host_request;
  if (!WaitForEvent(&state, 1000, &host_request)) {
    DestroyRuntimeState(&state);
    return ThrowError(env, "no host.request captured");
  }
  if (host_request.find("\"host.request\"") == std::string::npos) {
    DestroyRuntimeState(&state);
    return ThrowError(env, "expected host.request event");
  }

  uint64_t operation_id = 0;
  if (!ExtractUnsignedField(host_request, "operationId", &operation_id)) {
    DestroyRuntimeState(&state);
    return ThrowError(env, "host.request missing operationId");
  }

  std::string complete = BuildHostCompleteCommand(
      11, operation_id, "{\"status\":\"ok\",\"source\":\"harmony-napi\"}");
  code = rc_runtime_send(state.runtime,
                         reinterpret_cast<const uint8_t *>(complete.data()),
                         complete.size());
  if (code != 0) {
    DestroyRuntimeState(&state);
    return ThrowError(env, "rc_runtime_send host.complete failed");
  }

  std::string completion_event;
  if (!WaitForEvent(&state, 1000, &completion_event)) {
    DestroyRuntimeState(&state);
    return ThrowError(env, "no host.complete result captured");
  }

  DestroyRuntimeState(&state);

  std::string result = "{\"hostRequest\":" + host_request +
                       ",\"completion\":" + completion_event + "}";
  return StringResult(env, result, "failed to create host smoke result value");
}

napi_value Init(napi_env env, napi_value exports) {
  napi_property_descriptor properties[] = {
      {"abiVersion", nullptr, AbiVersion, nullptr, nullptr, nullptr, napi_default,
       nullptr},
      {"createRuntime", nullptr, CreateRuntime, nullptr, nullptr, nullptr,
       napi_default, nullptr},
      {"releaseRuntime", nullptr, ReleaseRuntime, nullptr, nullptr, nullptr,
       napi_default, nullptr},
      {"sendCommand", nullptr, SendCommand, nullptr, nullptr, nullptr,
       napi_default, nullptr},
      {"cancelRequest", nullptr, CancelRequest, nullptr, nullptr, nullptr,
       napi_default, nullptr},
      {"readEvent", nullptr, ReadEvent, nullptr, nullptr, nullptr, napi_default,
       nullptr},
      {"pendingEventCount", nullptr, PendingEventCount, nullptr, nullptr,
       nullptr, napi_default, nullptr},
      {"completeHostRequest", nullptr, CompleteHostRequest, nullptr, nullptr,
       nullptr, napi_default, nullptr},
      {"failHostRequest", nullptr, FailHostRequest, nullptr, nullptr, nullptr,
       napi_default, nullptr},
      {"pingSmoke", nullptr, PingSmoke, nullptr, nullptr, nullptr, napi_default,
       nullptr},
      {"hostSmoke", nullptr, HostSmoke, nullptr, nullptr, nullptr, napi_default,
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
