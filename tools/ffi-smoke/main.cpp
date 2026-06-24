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

  // --- Create rejection paths -------------------------------------------
  if (rc_runtime_create(nullptr, 0, capture_event, nullptr, nullptr) != 2) {
    return fail("null out_runtime did not return status 2");
  }
  rc_runtime_t *no_runtime = nullptr;
  if (rc_runtime_create(nullptr, 0, nullptr, nullptr, &no_runtime) != 3 ||
      no_runtime != nullptr) {
    return fail("null callback did not return status 3");
  }
  if (rc_runtime_create(reinterpret_cast<const uint8_t *>("{not json"), 9,
                        capture_event, nullptr, &no_runtime) != 4) {
    return fail("invalid config did not return status 4");
  }
  int32_t code = 0;
  auto msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || msg.empty()) {
    std::cerr << "invalid config last_error: code=" << code << " msg=" << msg
              << '\n';
    return fail("invalid config did not record INVALID_MESSAGE");
  }

  // --- Create a real runtime --------------------------------------------
  Channel ch;
  rc_runtime_t *rt = nullptr;
  std::string config = "{}";
  code = rc_runtime_create(reinterpret_cast<const uint8_t *>(config.data()),
                           config.size(), capture_event, &ch, &rt);
  if (code != 0 || rt == nullptr) {
    std::cerr << "rc_runtime_create failed: " << code << '\n';
    return 1;
  }
  if (rc_last_error(nullptr, 0) != RC_OK) {
    return fail("successful create did not clear last_error");
  }

  // --- Synchronous send failures ----------------------------------------
  if (send_str(rt, "{") != 3) {
    return fail("malformed send did not return status 3");
  }
  msg = last_error_message(&code);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    return fail("malformed send did not record INVALID_MESSAGE");
  }
  std::string proto_v2 =
      R"({"protocolVersion":2,"requestId":9,"method":"runtime.ping","params":{}})";
  if (send_str(rt, proto_v2) != 4) {
    return fail("protocol v2 send did not return status 4");
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
      0) {
    return fail("core.info send failed");
  }
  auto event = wait_event(ch, ev++);
  if (!contains(event, "\"requestId\":10") || !contains(event, "capabilities")) {
    return fail("core.info event shape");
  }

  // --- host.request -> host.complete ------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":20,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{"hello":"world"}}})") !=
      0) {
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
  if (send_str(rt, complete) != 0) {
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
      0) {
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
  if (send_str(rt, err_cmd) != 0) {
    return fail("host.error(22) send failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":22") ||
      !contains(event, "\"INTERNAL\"")) {
    std::cerr << "error(22): " << event << '\n';
    return fail("host.error result shape");
  }

  // --- cancel a pending host.request ------------------------------------
  if (send_str(rt,
               R"({"protocolVersion":1,"requestId":24,"method":"runtime.hostSmoke","params":{"capability":"host.smoke.echo","params":{}}})") !=
      0) {
    return fail("hostSmoke(24) send failed");
  }
  event = wait_event(ch, ev++);
  if (rc_runtime_cancel(rt, 24) != 0) {
    return fail("cancel(24) failed");
  }
  event = wait_event(ch, ev++);
  if (!contains(event, "\"requestId\":24") ||
      !contains(event, "\"CANCELLED\"")) {
    return fail("cancelled(24) shape");
  }

  // Last successful send/cancel cleared the error slot.
  if (rc_last_error(nullptr, 0) != RC_OK) {
    return fail("successful cancel did not clear last_error");
  }

  rc_runtime_destroy(rt);

  std::cout << "c-abi-smoke-cxx: ok\n";
  return 0;
}
