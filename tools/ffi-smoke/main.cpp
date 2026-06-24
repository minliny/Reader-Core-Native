#include "reader_core.h"

#include <chrono>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <mutex>
#include <string>
#include <thread>

namespace {

struct CapturedEvent {
  std::mutex mutex;
  std::string json;
};

void capture_event(void *context, const uint8_t *json, size_t json_length) {
  auto *captured = static_cast<CapturedEvent *>(context);
  std::lock_guard<std::mutex> lock(captured->mutex);
  captured->json.assign(reinterpret_cast<const char *>(json), json_length);
}

bool contains(const std::string &value, const char *needle) {
  return value.find(needle) != std::string::npos;
}

} // namespace

int main() {
  if (rc_abi_version() != 1) {
    std::cerr << "unexpected ABI version: " << rc_abi_version() << '\n';
    return 1;
  }

  rc_runtime_t *invalid_runtime = nullptr;
  int32_t code = rc_runtime_create(nullptr, 0, capture_event, nullptr, nullptr);
  if (code != 2) {
    std::cerr << "invalid out pointer returned " << code << ", expected 2\n";
    return 1;
  }
  code = rc_runtime_create(nullptr, 0, nullptr, nullptr, &invalid_runtime);
  if (code != 3 || invalid_runtime != nullptr) {
    std::cerr << "missing callback returned " << code << ", expected 3\n";
    return 1;
  }

  CapturedEvent captured;
  rc_runtime_t *runtime = nullptr;
  const std::string config = "{}";
  code = rc_runtime_create(
      reinterpret_cast<const uint8_t *>(config.data()), config.size(),
      capture_event, &captured, &runtime);
  if (code != 0 || runtime == nullptr) {
    std::cerr << "rc_runtime_create failed: " << code << '\n';
    return 1;
  }

  const std::string command =
      R"({"protocolVersion":1,"requestId":77,"method":"runtime.ping","params":{}})";
  code = rc_runtime_send(runtime,
                         reinterpret_cast<const uint8_t *>(command.data()),
                         command.size());
  if (code != 0) {
    std::cerr << "rc_runtime_send failed: " << code << '\n';
    rc_runtime_destroy(runtime);
    return 1;
  }

  code = rc_runtime_cancel(runtime, 999);
  if (code != 0) {
    std::cerr << "rc_runtime_cancel failed for unknown request: " << code
              << '\n';
    rc_runtime_destroy(runtime);
    return 1;
  }

  std::string event;
  for (int i = 0; i < 100; ++i) {
    {
      std::lock_guard<std::mutex> lock(captured.mutex);
      event = captured.json;
    }
    if (!event.empty()) {
      break;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
  }

  rc_runtime_destroy(runtime);

  if (event.empty()) {
    std::cerr << "no event captured\n";
    return 1;
  }
  if (!contains(event, R"("requestId":77)") ||
      !contains(event, R"("pong":true)")) {
    std::cerr << "unexpected event: " << event << '\n';
    return 1;
  }

  std::cout << event << '\n';
  return 0;
}
