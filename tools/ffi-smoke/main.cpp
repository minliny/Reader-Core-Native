// C++ ABI smoke for reader_core.h.
//
// Mirrors the C smoke in C++: drives create/send/event/host-bus/cancel and
// reads structured errors via rc_last_error. Failure paths are covered.

#include "reader_core.h"

#include <chrono>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

static_assert(RC_CREATE_PANIC == -1, "RC_CREATE_PANIC changed");
static_assert(RC_CREATE_OK == 0, "RC_CREATE_OK changed");
static_assert(RC_CREATE_NULL_OUT_RUNTIME == 2,
              "RC_CREATE_NULL_OUT_RUNTIME changed");
static_assert(RC_CREATE_NULL_CALLBACK == 3,
              "RC_CREATE_NULL_CALLBACK changed");
static_assert(RC_CREATE_INVALID_CONFIG == 4,
              "RC_CREATE_INVALID_CONFIG changed");
static_assert(RC_SEND_PANIC == -1, "RC_SEND_PANIC changed");
static_assert(RC_SEND_OK == 0, "RC_SEND_OK changed");
static_assert(RC_SEND_NULL_RUNTIME == 1, "RC_SEND_NULL_RUNTIME changed");
static_assert(RC_SEND_NULL_COMMAND == 2, "RC_SEND_NULL_COMMAND changed");
static_assert(RC_SEND_INVALID_COMMAND == 3,
              "RC_SEND_INVALID_COMMAND changed");
static_assert(RC_SEND_PROTOCOL_ERROR == 4,
              "RC_SEND_PROTOCOL_ERROR changed");
static_assert(RC_CANCEL_PANIC == -1, "RC_CANCEL_PANIC changed");
static_assert(RC_CANCEL_OK == 0, "RC_CANCEL_OK changed");
static_assert(RC_CANCEL_NULL_RUNTIME == 1, "RC_CANCEL_NULL_RUNTIME changed");
static_assert(RC_OK == 0, "RC_OK changed");
static_assert(RC_ERR_UNKNOWN_METHOD == 1, "RC_ERR_UNKNOWN_METHOD changed");
static_assert(RC_ERR_INVALID_PARAMS == 2, "RC_ERR_INVALID_PARAMS changed");
static_assert(RC_ERR_INVALID_PROTOCOL_VERSION == 3,
              "RC_ERR_INVALID_PROTOCOL_VERSION changed");
static_assert(RC_ERR_CANCELLED == 4, "RC_ERR_CANCELLED changed");
static_assert(RC_ERR_INVALID_MESSAGE == 5, "RC_ERR_INVALID_MESSAGE changed");
static_assert(RC_ERR_INTERNAL == 6, "RC_ERR_INTERNAL changed");

namespace {

constexpr size_t kMaxEvents = 80;

struct Channel {
  std::mutex mutex;
  std::vector<std::string> events;
};

void capture_event(void *context, const uint8_t *json, size_t json_length) {
  auto *ch = static_cast<Channel *>(context);
  std::lock_guard<std::mutex> lock(ch->mutex);
  if (ch->events.size() < kMaxEvents) {
    ch->events.emplace_back(reinterpret_cast<const char *>(json), json_length);
  }
}

size_t channel_count(Channel &ch) {
  std::lock_guard<std::mutex> lock(ch.mutex);
  return ch.events.size();
}

// Wait until at least index+1 events have arrived, then return event[index].
std::string wait_event(Channel &ch, size_t index) {
  for (int i = 0; i < 1000; ++i) {
    if (channel_count(ch) > index) {
      break;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  std::lock_guard<std::mutex> lock(ch.mutex);
  if (index >= ch.events.size()) {
    return {};
  }
  return ch.events[index];
}

bool contains(const std::string &haystack, const char *needle) {
  return haystack.find(needle) != std::string::npos;
}

size_t count_occurrences(const std::string &haystack, const char *needle) {
  size_t count = 0;
  size_t pos = 0;
  const std::string needle_str(needle);
  while ((pos = haystack.find(needle_str, pos)) != std::string::npos) {
    ++count;
    pos += needle_str.size();
  }
  return count;
}

int send_str(rc_runtime_t *rt, const std::string &json) {
  return rc_runtime_send(rt, reinterpret_cast<const uint8_t *>(json.data()),
                         json.size());
}

std::string last_error_message(int32_t *code_out) {
  char buf[256] = {};
  int32_t code = rc_last_error(buf, sizeof buf);
  if (code_out) {
    *code_out = code;
  }
  return std::string(buf);
}

bool last_error_clears_message_when_ok() {
  char buf[16] = "stale";
  return rc_last_error(buf, sizeof buf) == RC_OK && buf[0] == '\0';
}

// Extract a "key":<uint64> value from a JSON string. Returns false on miss.
bool json_u64(const std::string &json, const char *key, uint64_t *out) {
  std::string needle = "\"" + std::string(key) + "\":";
  auto pos = json.find(needle);
  if (pos == std::string::npos) {
    return false;
  }
  pos += needle.size();
  while (pos < json.size() &&
         (json[pos] == ' ' || json[pos] == '\t' || json[pos] == '\n')) {
    ++pos;
  }
  try {
    *out = std::stoull(json.substr(pos));
  } catch (...) {
    return false;
  }
  return true;
}

int fail(const char *msg) {
  std::cerr << "FAIL: " << msg << '\n';
  return 1;
}

} // namespace

int main() {
  if (rc_abi_version() != 1) {
    std::cerr << "unexpected ABI version: " << rc_abi_version() << '\n';
    return 1;
  }
  int32_t code = RC_OK;
  std::string msg;

  // --- Failure paths that need no runtime -------------------------------
  if (rc_runtime_send(nullptr, reinterpret_cast<const uint8_t *>("{}"), 2) !=
      RC_SEND_NULL_RUNTIME) {
    return fail("null runtime send did not return RC_SEND_NULL_RUNTIME");
  }
  code = rc_last_error(nullptr, 16);
  if (code != RC_ERR_INVALID_MESSAGE) {
    return fail("null-buffer last_error did not return INVALID_MESSAGE");
  }
  char zero_cap[] = "stale";
  code = rc_last_error(zero_cap, 0);
  if (code != RC_ERR_INVALID_MESSAGE || std::string(zero_cap) != "stale") {
    return fail("zero-cap last_error wrote message or changed code");
  }
  (void)rc_abi_version();
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "runtime handle")) {
    return fail("rc_abi_version touched last_error");
  }
  if (rc_runtime_cancel(nullptr, 42) != RC_CANCEL_NULL_RUNTIME) {
    return fail("null runtime cancel did not return RC_CANCEL_NULL_RUNTIME");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "runtime handle")) {
    return fail("null runtime cancel did not record INVALID_MESSAGE");
  }
  rc_runtime_destroy(nullptr);
  if (!last_error_clears_message_when_ok()) {
    return fail("null destroy did not clear last_error");
  }

  // --- Create rejection paths -------------------------------------------
  if (rc_runtime_create(nullptr, 0, capture_event, nullptr, nullptr) !=
      RC_CREATE_NULL_OUT_RUNTIME) {
    return fail("null out_runtime did not return RC_CREATE_NULL_OUT_RUNTIME");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "out_runtime")) {
    return fail("null out_runtime did not record INVALID_MESSAGE");
  }
  rc_runtime_t *no_runtime = nullptr;
  auto *sentinel = reinterpret_cast<rc_runtime_t *>(static_cast<uintptr_t>(1));
  if (rc_runtime_create(nullptr, 0, nullptr, nullptr, &no_runtime) !=
          RC_CREATE_NULL_CALLBACK ||
      no_runtime != nullptr) {
    return fail("null callback did not return RC_CREATE_NULL_CALLBACK");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "event callback")) {
    return fail("null callback did not record INVALID_MESSAGE");
  }
  if (rc_runtime_create(nullptr, 0, nullptr, nullptr, &sentinel) !=
          RC_CREATE_NULL_CALLBACK ||
      sentinel != nullptr) {
    return fail("create failure did not clear out_runtime");
  }
  sentinel = reinterpret_cast<rc_runtime_t *>(static_cast<uintptr_t>(1));
  if (rc_runtime_create(nullptr, 1, capture_event, nullptr, &sentinel) !=
          RC_CREATE_INVALID_CONFIG ||
      sentinel != nullptr) {
    return fail("null config with non-zero length did not return RC_CREATE_INVALID_CONFIG");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "config_json")) {
    return fail("null config did not record INVALID_MESSAGE");
  }
  sentinel = reinterpret_cast<rc_runtime_t *>(static_cast<uintptr_t>(1));
  if (rc_runtime_create(reinterpret_cast<const uint8_t *>("{not json"), 9,
                        capture_event, nullptr, &sentinel) !=
          RC_CREATE_INVALID_CONFIG ||
      sentinel != nullptr) {
    return fail("invalid config did not return RC_CREATE_INVALID_CONFIG");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || msg.empty()) {
    std::cerr << "invalid config last_error: code=" << code << " msg=" << msg
              << '\n';
    return fail("invalid config did not record INVALID_MESSAGE");
  }
  std::string unknown_field_config =
      R"({"dataDirectory":"/tmp/reader-core-native/data","extraDirectory":"/tmp/reader-core-native/extra"})";
  sentinel = reinterpret_cast<rc_runtime_t *>(static_cast<uintptr_t>(1));
  if (rc_runtime_create(
          reinterpret_cast<const uint8_t *>(unknown_field_config.data()),
          unknown_field_config.size(), capture_event, nullptr, &sentinel) !=
          RC_CREATE_INVALID_CONFIG ||
      sentinel != nullptr) {
    return fail("unknown field config did not return RC_CREATE_INVALID_CONFIG");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "runtime config")) {
    std::cerr << "unknown field config last_error: code=" << code
              << " msg=" << msg << '\n';
    return fail("unknown field config did not record INVALID_MESSAGE");
  }
  std::string empty_data_dir_config = R"({"dataDirectory":""})";
  sentinel = reinterpret_cast<rc_runtime_t *>(static_cast<uintptr_t>(1));
  if (rc_runtime_create(
          reinterpret_cast<const uint8_t *>(empty_data_dir_config.data()),
          empty_data_dir_config.size(), capture_event, nullptr, &sentinel) !=
          RC_CREATE_INVALID_CONFIG ||
      sentinel != nullptr) {
    return fail("empty dataDirectory config did not return RC_CREATE_INVALID_CONFIG");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_PARAMS || !contains(msg, "dataDirectory")) {
    std::cerr << "empty dataDirectory last_error: code=" << code
              << " msg=" << msg << '\n';
    return fail("empty dataDirectory config did not record INVALID_PARAMS");
  }

  Channel defaults_ch;
  rc_runtime_t *defaults_rt = nullptr;
  if (rc_runtime_create(nullptr, 0, capture_event, &defaults_ch, &defaults_rt) !=
          RC_CREATE_OK ||
      defaults_rt == nullptr) {
    return fail("null config with zero length did not create defaults runtime");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("default create did not clear last_error");
  }
  rc_runtime_destroy(defaults_rt);

  // --- Create a real runtime --------------------------------------------
  Channel ch;
  rc_runtime_t *rt = nullptr;
  std::string config = "{}";
  code = rc_runtime_create(reinterpret_cast<const uint8_t *>(config.data()),
                           config.size(), capture_event, &ch, &rt);
  if (code != RC_CREATE_OK || rt == nullptr) {
    std::cerr << "rc_runtime_create failed: " << code << '\n';
    return 1;
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("successful create did not clear last_error");
  }

  // --- Synchronous send failures ----------------------------------------
  if (rc_runtime_send(rt, nullptr, 1) != RC_SEND_NULL_COMMAND) {
    return fail("null command with non-zero length did not return RC_SEND_NULL_COMMAND");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command_json")) {
    return fail("null command did not record INVALID_MESSAGE");
  }
  if (rc_runtime_cancel(rt, 123456) != RC_CANCEL_OK) {
    return fail("cancel missing request did not return RC_CANCEL_OK");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("cancel missing request did not clear last_error");
  }
  if (channel_count(ch) != 0) {
    return fail("cancel missing request emitted an async event");
  }

  if (rc_runtime_send(rt, nullptr, 0) != RC_SEND_INVALID_COMMAND) {
    return fail("zero-length command did not return RC_SEND_INVALID_COMMAND");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    return fail("zero-length command did not record INVALID_MESSAGE");
  }

  if (rc_runtime_send(rt, reinterpret_cast<const uint8_t *>(""), 0) !=
      RC_SEND_INVALID_COMMAND) {
    return fail("empty command did not return RC_SEND_INVALID_COMMAND");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    return fail("empty command did not record INVALID_MESSAGE");
  }

  if (send_str(rt, "{") != RC_SEND_INVALID_COMMAND) {
    return fail("malformed send did not return RC_SEND_INVALID_COMMAND");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    return fail("malformed send did not record INVALID_MESSAGE");
  }
  std::string params_not_object =
      R"({"protocolVersion":1,"requestId":8,"method":"runtime.ping","params":[]})";
  if (send_str(rt, params_not_object) != RC_SEND_INVALID_COMMAND) {
    return fail("non-object params did not return RC_SEND_INVALID_COMMAND");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_PARAMS || !contains(msg, "params")) {
    return fail("non-object params did not record INVALID_PARAMS");
  }
  if (channel_count(ch) != 0) {
    return fail("non-object params emitted an async event");
  }
  std::string proto_v2 =
      R"({"protocolVersion":2,"requestId":9,"method":"runtime.ping","params":{}})";
  if (send_str(rt, proto_v2) != RC_SEND_PROTOCOL_ERROR) {
    return fail("protocol v2 send did not return RC_SEND_PROTOCOL_ERROR");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_PROTOCOL_VERSION ||
      !contains(msg, "protocolVersion")) {
    return fail("protocol v2 did not record INVALID_PROTOCOL_VERSION");
  }

  std::string missing_request_id =
      R"({"protocolVersion":1,"method":"runtime.ping","params":{}})";
  if (send_str(rt, missing_request_id) != RC_SEND_INVALID_COMMAND) {
    return fail("missing requestId did not return RC_SEND_INVALID_COMMAND");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    return fail("missing requestId did not record INVALID_MESSAGE");
  }

  std::string request_id_zero =
      R"({"protocolVersion":1,"requestId":0,"method":"runtime.ping","params":{}})";
  if (send_str(rt, request_id_zero) != RC_SEND_PROTOCOL_ERROR) {
    return fail("requestId zero did not return RC_SEND_PROTOCOL_ERROR");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "requestId")) {
    return fail("requestId zero did not record INVALID_MESSAGE");
  }

  std::string empty_method =
      R"({"protocolVersion":1,"requestId":205,"method":"","params":{}})";
  if (send_str(rt, empty_method) != RC_SEND_PROTOCOL_ERROR) {
    return fail("empty method did not return RC_SEND_PROTOCOL_ERROR");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "method")) {
    return fail("empty method did not record INVALID_MESSAGE");
  }

  std::string method_whitespace =
      R"({"protocolVersion":1,"requestId":206,"method":"runtime. ping","params":{}})";
  if (send_str(rt, method_whitespace) != RC_SEND_PROTOCOL_ERROR) {
    return fail("method whitespace did not return RC_SEND_PROTOCOL_ERROR");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "method")) {
    return fail("method whitespace did not record INVALID_MESSAGE");
  }

  std::string method_empty_segment =
      R"({"protocolVersion":1,"requestId":207,"method":"runtime..ping","params":{}})";
  if (send_str(rt, method_empty_segment) != RC_SEND_PROTOCOL_ERROR) {
    return fail("method empty segment did not return RC_SEND_PROTOCOL_ERROR");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "method")) {
    return fail("method empty segment did not record INVALID_MESSAGE");
  }
  if (channel_count(ch) != 0) {
    return fail("invalid command envelope emitted an async event");
  }

  size_t ev = 0;

  // --- core.info ---------------------------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":10,"method":"core.info","params":{}})") !=
      RC_SEND_OK) {
    return fail("core.info send failed");
  }
  auto event = wait_event(ch, ev++);
  const char *core_info_needles[] = {
      "\"type\":\"result\"",
      "\"requestId\":10",
      "\"abiVersion\":1",
      "\"buildVersion\":\"reader-core-native ",
      "\"capabilities\":[",
      "\"core.info\"",
      "\"runtime.ping\"",
      "\"runtime.hostSmoke\"",
      "\"runtime.cancel\"",
      "\"runtime.status\"",
      "\"runtime.shutdown\"",
      "\"host.complete\"",
      "\"host.error\"",
      "\"host.bus.v1\"",
      "\"http.execute\"",
      "\"runtime.config.v1\"",
      "\"remote.reading.v1\"",
  };
  for (const char *needle : core_info_needles) {
    if (!contains(event, needle)) {
      std::cerr << "core.info missing " << needle << " in event: " << event
                << '\n';
      return fail("core.info event shape");
    }
  }
  if (count_occurrences(event, "\"protocolVersion\":1") < 2) {
    std::cerr << "unexpected core.info event: " << event << '\n';
    return fail("core.info did not report both event and data protocolVersion");
  }

  // --- host.request -> host.complete ------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":20,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"hello":"world"}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(20) send failed");
  }
  event = wait_event(ch, ev++);
  uint64_t op = 0;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":20") ||
      !contains(event, "\"capability\":\"host.smoke.echo\"") ||
      !contains(event, "\"hello\":\"world\"") ||
      !json_u64(event, "operationId", &op)) {
    std::cerr << "host.request(20): " << event << '\n';
    return fail("host.request(20) shape");
  }
  std::string complete =
      R"({"protocolVersion":1,"requestId":21,"method":"host.complete","params":{"operationId":)" +
      std::to_string(op) + R"(,"result":{"echoed":true}}})";
  if (send_str(rt, complete) != RC_SEND_OK) {
    return fail("host.complete(20) send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":20") ||
      !contains(event, "\"echoed\":true")) {
    std::cerr << "result(20): " << event << '\n';
    return fail("host.complete result shape");
  }
  if (rc_runtime_cancel(rt, 20) != RC_CANCEL_OK ||
      rc_runtime_cancel(rt, 20) != RC_CANCEL_OK) {
    return fail("cancel completed request did not return RC_CANCEL_OK");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("cancel completed request did not clear last_error");
  }
  if (channel_count(ch) != ev) {
    return fail("cancel completed request emitted an async event");
  }
  std::string duplicate_complete =
      R"({"protocolVersion":1,"requestId":38,"method":"host.complete","params":{"operationId":)" +
      std::to_string(op) + R"(,"result":{"echoed":true}}})";
  if (send_str(rt, duplicate_complete) != RC_SEND_OK) {
    return fail("duplicate host.complete(20) send failed");
  }
  event = wait_event(ch, ev++);
  const std::string completed_op_field =
      "\"operationId\":" + std::to_string(op);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":38") ||
      !contains(event, "\"code\":\"INVALID_PARAMS\"") ||
      !contains(event, completed_op_field.c_str())) {
    std::cerr << "duplicate host.complete error: " << event << '\n';
    return fail("duplicate host.complete error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("duplicate host.complete left synchronous last_error");
  }

  // --- invalid runtime.hostSmoke params return async protocol errors ----
  struct InvalidHostRequest {
    const char *name;
    std::string json;
    uint64_t request_id;
    const char *message_fragment;
  };
  const InvalidHostRequest invalid_host_requests[] = {
      {"capability whitespace",
       R"({"protocolVersion":1,"requestId":307,"method":"runtime.hostSmoke","params":{"capability":"host. smoke.echo","params":{"message":"invalid capability"}}})",
       307,
       "capability"},
      {"capability empty segment",
       R"({"protocolVersion":1,"requestId":308,"method":"runtime.hostSmoke","params":{"capability":"host..echo","params":{"message":"invalid capability"}}})",
       308,
       "capability"},
      {"unknown params",
       R"({"protocolVersion":1,"requestId":309,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"message":"unexpected metadata"},"timeoutMs":1000}})",
       309,
       "runtime.hostSmoke"},
  };
  for (const auto &invalid : invalid_host_requests) {
    if (send_str(rt, invalid.json) != RC_SEND_OK) {
      return fail("invalid host request send failed");
    }
    if (!last_error_clears_message_when_ok()) {
      return fail("invalid host request left synchronous last_error");
    }
    event = wait_event(ch, ev++);
    const std::string request_id_field =
        "\"requestId\":" + std::to_string(invalid.request_id);
    if (!contains(event, "\"protocolVersion\":1") ||
        !contains(event, "\"type\":\"error\"") ||
        !contains(event, request_id_field.c_str()) ||
        !contains(event, "\"code\":\"INVALID_PARAMS\"") ||
        !contains(event, invalid.message_fragment) ||
        contains(event, "\"type\":\"host.request\"")) {
      std::cerr << "invalid host request " << invalid.name << ": " << event
                << '\n';
      return fail("invalid host request error shape");
    }
  }

  // --- host.request -> host.error ---------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":22,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(22) send failed");
  }
  event = wait_event(ch, ev++);
  if (!json_u64(event, "operationId", &op) ||
      !contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":22")) {
    return fail("host.request(22) shape");
  }
  std::string err_cmd =
      R"({"protocolVersion":1,"requestId":23,"method":"host.error","params":{"operationId":)" +
      std::to_string(op) +
      R"(,"error":{"code":"INTERNAL","message":"host failed","retryable":true}}})";
  if (send_str(rt, err_cmd) != RC_SEND_OK) {
    return fail("host.error(22) send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":22") ||
      !contains(event, "\"INTERNAL\"") ||
      !contains(event, "\"retryable\":true")) {
    std::cerr << "error(22): " << event << '\n';
    return fail("host.error result shape");
  }

  // --- runtime.status reports pending host metadata without payload -----
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":35,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"hidden":"status payload"}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(35) send failed");
  }
  event = wait_event(ch, ev++);
  uint64_t pending_op = 0;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":35") ||
      !contains(event, "\"capability\":\"host.smoke.echo\"") ||
      !contains(event, "\"hidden\":\"status payload\"") ||
      !json_u64(event, "operationId", &pending_op)) {
    std::cerr << "host.request(35): " << event << '\n';
    return fail("host.request(35) shape");
  }
  if (send_str(
          rt,
          R"({"protocolVersion":1,"requestId":36,"method":"runtime.status","params":{}})") !=
      RC_SEND_OK) {
    return fail("runtime.status send failed");
  }
  event = wait_event(ch, ev++);
  std::string pending_op_field =
      "\"operationId\":" + std::to_string(pending_op);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":36") ||
      !contains(event, "\"activeRequestCount\":1") ||
      !contains(event, "\"activeRequestIds\":[35]") ||
      !contains(event, "\"pendingHostOperationCount\":1") ||
      !contains(event, "\"pendingHostOperations\"") ||
      !contains(event, pending_op_field.c_str()) ||
      !contains(event, "\"requestId\":35") ||
      !contains(event, "\"capability\":\"host.smoke.echo\"") ||
      !contains(event, "\"state\":\"pending\"") ||
      !contains(event, "\"shuttingDown\":false") ||
      contains(event, "\"hidden\"") || contains(event, "status payload")) {
    std::cerr << "runtime.status result: " << event << '\n';
    return fail("runtime.status pending metadata shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("runtime.status left synchronous last_error");
  }
  std::string status_complete =
      R"({"protocolVersion":1,"requestId":37,"method":"host.complete","params":{"operationId":)" +
      std::to_string(pending_op) + R"(,"result":{"echoed":true}}})";
  if (send_str(rt, status_complete) != RC_SEND_OK) {
    return fail("host.complete(35) send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":35") ||
      !contains(event, "\"echoed\":true")) {
    std::cerr << "status host.complete result: " << event << '\n';
    return fail("status host.complete result shape");
  }

  // --- host.complete for an unknown operation -> async INVALID_PARAMS ----
  std::string unknown_complete =
      R"({"protocolVersion":1,"requestId":25,"method":"host.complete","params":{"operationId":999999,"result":{"ok":true}}})";
  if (send_str(rt, unknown_complete) != RC_SEND_OK) {
    return fail("unknown host.complete send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":25") ||
      !contains(event, "\"INVALID_PARAMS\"")) {
    std::cerr << "unknown host.complete error: " << event << '\n';
    return fail("unknown host.complete error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("unknown host.complete send left synchronous last_error");
  }

  // --- host.complete invalid params -> async INVALID_PARAMS -------------
  std::string zero_complete =
      R"({"protocolVersion":1,"requestId":305,"method":"host.complete","params":{"operationId":0,"result":{"status":"invalid"}}})";
  if (send_str(rt, zero_complete) != RC_SEND_OK) {
    return fail("zero operation host.complete send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":305") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "\"operationId\":0")) {
    std::cerr << "zero operation host.complete error: " << event << '\n';
    return fail("zero operation host.complete error shape");
  }
  std::string unknown_field_complete =
      R"({"protocolVersion":1,"requestId":314,"method":"host.complete","params":{"operationId":1,"result":{"status":"ok"},"completedAt":123}})";
  if (send_str(rt, unknown_field_complete) != RC_SEND_OK) {
    return fail("unknown field host.complete send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":314") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "host.complete") ||
      !contains(event, "unknown field")) {
    std::cerr << "unknown field host.complete error: " << event << '\n';
    return fail("unknown field host.complete error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("host.complete invalid params left synchronous last_error");
  }

  // --- host.error invalid params -> async INVALID_PARAMS ----------------
  std::string zero_error =
      R"({"protocolVersion":1,"requestId":306,"method":"host.error","params":{"operationId":0,"error":{"code":"INTERNAL","message":"invalid operation id","retryable":false}}})";
  if (send_str(rt, zero_error) != RC_SEND_OK) {
    return fail("zero operation host.error send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":306") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "\"operationId\":0")) {
    std::cerr << "zero operation host.error error: " << event << '\n';
    return fail("zero operation host.error error shape");
  }
  std::string unknown_field_error =
      R"({"protocolVersion":1,"requestId":315,"method":"host.error","params":{"operationId":1,"error":{"code":"INTERNAL","message":"host failed","retryable":true},"failedAt":123}})";
  if (send_str(rt, unknown_field_error) != RC_SEND_OK) {
    return fail("unknown field host.error send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":315") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "host.error") ||
      !contains(event, "unknown field")) {
    std::cerr << "unknown field host.error error: " << event << '\n';
    return fail("unknown field host.error error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("host.error invalid params left synchronous last_error");
  }

  // --- remote http.execute completion carries metadata ------------------
  std::string http_search =
      R"({"protocolVersion":1,"requestId":27,"method":"book.search","params":{"sourceId":"ffi-http-src","searchRequest":{"url":"https://books.example.test/search?q=abi","headers":{"Accept":"application/json"}},"source":{"sourceId":"ffi-http-src","name":"FFI HTTP Source","baseUrl":"https://books.example.test","rules":{"search":[{"kind":"jsonPath","path":"$.books[*]"}]}}}})";
  if (send_str(rt, http_search) != RC_SEND_OK) {
    return fail("book.search http request send failed");
  }
  event = wait_event(ch, ev++);
  uint64_t http_op = 0;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":27") ||
      !contains(event, "\"capability\":\"http.execute\"") ||
      !contains(event, "search?q=abi") ||
      !contains(event, "\"Accept\":\"application/json\"") ||
      !json_u64(event, "operationId", &http_op)) {
    std::cerr << "http.execute request: " << event << '\n';
    return fail("http.execute request shape");
  }
  std::string http_complete =
      R"({"protocolVersion":1,"requestId":28,"method":"host.complete","params":{"operationId":)" +
      std::to_string(http_op) +
      R"(,"result":{"status":200,"headers":{"content-type":"application/json"},"body":"{\"books\":[]}"}}})";
  if (send_str(rt, http_complete) != RC_SEND_OK) {
    return fail("http host.complete send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":27") ||
      !contains(event, "\"books\":[]") ||
      !contains(event, "\"http\"") ||
      !contains(event, "\"status\":200") ||
      !contains(event, "\"content-type\":\"application/json\"")) {
    std::cerr << "http completion result: " << event << '\n';
    return fail("http completion result shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("http completion left synchronous last_error");
  }

  // --- remote http.execute invalid status -> async INVALID_PARAMS -------
  std::string invalid_http_search =
      R"({"protocolVersion":1,"requestId":29,"method":"book.search","params":{"sourceId":"ffi-http-src","searchRequest":{"url":"https://books.example.test/search?q=invalid-status"},"source":{"sourceId":"ffi-http-src","name":"FFI HTTP Source","baseUrl":"https://books.example.test","rules":{"search":[{"kind":"jsonPath","path":"$.books[*]"}]}}}})";
  if (send_str(rt, invalid_http_search) != RC_SEND_OK) {
    return fail("book.search invalid-status request send failed");
  }
  event = wait_event(ch, ev++);
  uint64_t invalid_http_op = 0;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":29") ||
      !contains(event, "\"capability\":\"http.execute\"") ||
      !contains(event, "search?q=invalid-status") ||
      !json_u64(event, "operationId", &invalid_http_op)) {
    std::cerr << "invalid-status http.execute request: " << event << '\n';
    return fail("invalid-status http.execute request shape");
  }
  std::string invalid_http_complete =
      R"({"protocolVersion":1,"requestId":30,"method":"host.complete","params":{"operationId":)" +
      std::to_string(invalid_http_op) +
      R"(,"result":{"status":99,"body":"{\"books\":[]}"}}})";
  if (send_str(rt, invalid_http_complete) != RC_SEND_OK) {
    return fail("invalid-status http host.complete send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":29") ||
      !contains(event, "\"code\":\"INVALID_PARAMS\"") ||
      !contains(event, "status") || !contains(event, "\"status\":99")) {
    std::cerr << "invalid http status error: " << event << '\n';
    return fail("invalid http status error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("invalid http status left synchronous last_error");
  }

  // --- remote http.execute invalid headers -> async INVALID_PARAMS ------
  std::string invalid_headers_search =
      R"({"protocolVersion":1,"requestId":31,"method":"book.search","params":{"sourceId":"ffi-http-src","searchRequest":{"url":"https://books.example.test/search?q=invalid-headers"},"source":{"sourceId":"ffi-http-src","name":"FFI HTTP Source","baseUrl":"https://books.example.test","rules":{"search":[{"kind":"jsonPath","path":"$.books[*]"}]}}}})";
  if (send_str(rt, invalid_headers_search) != RC_SEND_OK) {
    return fail("book.search invalid-headers request send failed");
  }
  event = wait_event(ch, ev++);
  uint64_t invalid_headers_op = 0;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":31") ||
      !contains(event, "\"capability\":\"http.execute\"") ||
      !contains(event, "search?q=invalid-headers") ||
      !json_u64(event, "operationId", &invalid_headers_op)) {
    std::cerr << "invalid-headers http.execute request: " << event << '\n';
    return fail("invalid-headers http.execute request shape");
  }
  std::string invalid_headers_complete =
      R"({"protocolVersion":1,"requestId":32,"method":"host.complete","params":{"operationId":)" +
      std::to_string(invalid_headers_op) +
      R"(,"result":{"status":200,"headers":["content-type","application/json"],"body":"{\"books\":[]}"}}})";
  if (send_str(rt, invalid_headers_complete) != RC_SEND_OK) {
    return fail("invalid-headers http host.complete send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":31") ||
      !contains(event, "\"code\":\"INVALID_PARAMS\"") ||
      !contains(event, "headers") ||
      !contains(event, "[\"content-type\",\"application/json\"]")) {
    std::cerr << "invalid http headers error: " << event << '\n';
    return fail("invalid http headers error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("invalid http headers left synchronous last_error");
  }

  // --- method-specific invalid params -> async INVALID_PARAMS ------------
  std::string invalid_progress =
      R"({"protocolVersion":1,"requestId":26,"method":"reading.progress.update","params":{"bookId":"1","chapterIndex":2,"chapterOffset":128,"chapterProgress":0.5,"syncToken":"host-owned"}})";
  if (send_str(rt, invalid_progress) != RC_SEND_OK) {
    return fail("reading.progress.update invalid params send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":26") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "reading.progress.update")) {
    std::cerr << "invalid reading.progress.update error: " << event << '\n';
    return fail("invalid reading.progress.update error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("invalid reading.progress.update send left synchronous last_error");
  }

  // --- cancel a pending host.request ------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":24,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(24) send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":24")) {
    std::cerr << "host.request(24): " << event << '\n';
    return fail("host.request(24) shape");
  }
  if (rc_runtime_cancel(rt, 24) != RC_CANCEL_OK) {
    return fail("cancel(24) failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"requestId\":24") ||
      !contains(event, "\"CANCELLED\"")) {
    return fail("cancelled(24) shape");
  }

  // Last successful send/cancel cleared the error slot.
  if (!last_error_clears_message_when_ok()) {
    return fail("successful cancel did not clear last_error");
  }

  // --- runtime.cancel command cancels a pending host.request ------------
  if (send_str(
          rt,
          R"({"protocolVersion":1,"requestId":301,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"message":"conformance host request"}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(301) send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":301") ||
      !contains(event, "\"capability\":\"host.smoke.echo\"") ||
      !contains(event, "\"message\":\"conformance host request\"")) {
    std::cerr << "host.request(301): " << event << '\n';
    return fail("host.request(301) shape");
  }
  if (send_str(
          rt,
          R"({"protocolVersion":1,"requestId":310,"method":"runtime.cancel","params":{"requestId":301}})") !=
      RC_SEND_OK) {
    return fail("runtime.cancel send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":301") ||
      !contains(event, "\"CANCELLED\"")) {
    std::cerr << "runtime.cancel cancelled event: " << event << '\n';
    return fail("runtime.cancel cancelled event shape");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":310") ||
      !contains(event, "\"cancelled\":true")) {
    std::cerr << "runtime.cancel result: " << event << '\n';
    return fail("runtime.cancel result shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("runtime.cancel left synchronous last_error");
  }

  // --- runtime.cancel command false/invalid params ----------------------
  if (send_str(
          rt,
          R"({"protocolVersion":1,"requestId":313,"method":"runtime.cancel","params":{"requestId":999999}})") !=
      RC_SEND_OK) {
    return fail("runtime.cancel unknown target send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":313") ||
      !contains(event, "\"cancelled\":false")) {
    std::cerr << "runtime.cancel false result: " << event << '\n';
    return fail("runtime.cancel false result shape");
  }
  if (send_str(
          rt,
          R"({"protocolVersion":1,"requestId":311,"method":"runtime.cancel","params":{"requestId":0}})") !=
      RC_SEND_OK) {
    return fail("runtime.cancel zero target send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":311") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "\"requestId\":0")) {
    std::cerr << "runtime.cancel zero target error: " << event << '\n';
    return fail("runtime.cancel zero target error shape");
  }
  if (send_str(
          rt,
          R"({"protocolVersion":1,"requestId":312,"method":"runtime.cancel","params":{"requestId":301,"reason":"host-request-timeout"}})") !=
      RC_SEND_OK) {
    return fail("runtime.cancel unknown field send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":312") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "runtime.cancel") ||
      !contains(event, "unknown field")) {
    std::cerr << "runtime.cancel unknown field error: " << event << '\n';
    return fail("runtime.cancel unknown field error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("runtime.cancel invalid params left synchronous last_error");
  }

  // --- invalid runtime.shutdown params do not stop runtime --------------
  std::string invalid_shutdown =
      R"({"protocolVersion":1,"requestId":33,"method":"runtime.shutdown","params":{"force":true}})";
  if (send_str(rt, invalid_shutdown) != RC_SEND_OK) {
    return fail("invalid runtime.shutdown send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":33") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "runtime.shutdown")) {
    std::cerr << "invalid runtime.shutdown error: " << event << '\n';
    return fail("invalid runtime.shutdown error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("invalid runtime.shutdown send left synchronous last_error");
  }

  if (send_str(
          rt,
          R"({"protocolVersion":1,"requestId":34,"method":"runtime.ping","params":{}})") !=
      RC_SEND_OK) {
    return fail("runtime.ping after invalid shutdown failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":34") ||
      !contains(event, "\"pong\":true")) {
    std::cerr << "ping after invalid shutdown: " << event << '\n';
    return fail("ping after invalid shutdown shape");
  }

  // --- runtime.shutdown cancels pending work and blocks future sends -----
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":30,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(30) send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":30")) {
    std::cerr << "host.request(30): " << event << '\n';
    return fail("host.request(30) shape");
  }
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":31,"method":"runtime.shutdown","params":{}})") !=
      RC_SEND_OK) {
    return fail("runtime.shutdown send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":30") ||
      !contains(event, "\"CANCELLED\"")) {
    std::cerr << "shutdown cancelled event: " << event << '\n';
    return fail("shutdown cancelled event shape");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":31") ||
      !contains(event, "\"shuttingDown\":true") ||
      !contains(event, "\"cancelledRequestIds\":[30]")) {
    std::cerr << "shutdown result: " << event << '\n';
    return fail("shutdown result shape");
  }
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":32,"method":"runtime.ping","params":{}})") !=
      RC_SEND_PROTOCOL_ERROR) {
    return fail("runtime.ping after shutdown did not return RC_SEND_PROTOCOL_ERROR");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INTERNAL || !contains(msg, "shutting down")) {
    return fail("post-shutdown send did not record INTERNAL");
  }
  if (channel_count(ch) != ev) {
    return fail("post-shutdown send emitted an async event");
  }

  if (send_str(rt, "{") != RC_SEND_INVALID_COMMAND) {
    return fail("pre-destroy invalid command did not return RC_SEND_INVALID_COMMAND");
  }
  rc_runtime_destroy(rt);
  if (!last_error_clears_message_when_ok()) {
    return fail("successful destroy did not clear last_error");
  }

  std::cout << "c-abi-smoke-cxx: ok\n";
  return 0;
}
