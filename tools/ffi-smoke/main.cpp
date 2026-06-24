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

constexpr size_t kMaxEvents = 32;

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

  size_t ev = 0;

  // --- core.info ---------------------------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":10,"method":"core.info","params":{}})") !=
      RC_SEND_OK) {
    return fail("core.info send failed");
  }
  auto event = wait_event(ch, ev++);
  if (!contains(event, "\"requestId\":10") || !contains(event, "capabilities")) {
    return fail("core.info event shape");
  }

  // --- host.request -> host.complete ------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":20,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"hello":"world"}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(20) send failed");
  }
  event = wait_event(ch, ev++);
  uint64_t op = 0;
  if (!contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":20") ||
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
  if (!contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":20") ||
      !contains(event, "\"echoed\":true")) {
    std::cerr << "result(20): " << event << '\n';
    return fail("host.complete result shape");
  }

  // --- host.request -> host.error ---------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":22,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(22) send failed");
  }
  event = wait_event(ch, ev++);
  if (!json_u64(event, "operationId", &op)) {
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
  if (!contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":22") ||
      !contains(event, "\"INTERNAL\"")) {
    std::cerr << "error(22): " << event << '\n';
    return fail("host.error result shape");
  }

  // --- host.complete for an unknown operation -> async INVALID_PARAMS ----
  std::string unknown_complete =
      R"({"protocolVersion":1,"requestId":25,"method":"host.complete","params":{"operationId":999999,"result":{"ok":true}}})";
  if (send_str(rt, unknown_complete) != RC_SEND_OK) {
    return fail("unknown host.complete send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":25") ||
      !contains(event, "\"INVALID_PARAMS\"")) {
    std::cerr << "unknown host.complete error: " << event << '\n';
    return fail("unknown host.complete error shape");
  }
  if (!last_error_clears_message_when_ok()) {
    return fail("unknown host.complete send left synchronous last_error");
  }

  // --- cancel a pending host.request ------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":24,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}})") !=
      RC_SEND_OK) {
    return fail("hostSmoke(24) send failed");
  }
  event = wait_event(ch, ev++);
  if (rc_runtime_cancel(rt, 24) != RC_CANCEL_OK) {
    return fail("cancel(24) failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"requestId\":24") ||
      !contains(event, "\"CANCELLED\"")) {
    return fail("cancelled(24) shape");
  }

  // Last successful send/cancel cleared the error slot.
  if (!last_error_clears_message_when_ok()) {
    return fail("successful cancel did not clear last_error");
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
