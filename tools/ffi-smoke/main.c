#include "reader_core.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

struct captured_event {
  char json[4096];
  size_t length;
};

static void capture_event(void *context, const uint8_t *json, size_t json_length) {
  struct captured_event *captured = (struct captured_event *)context;
  size_t copy_length = json_length;
  if (copy_length >= sizeof(captured->json)) {
    copy_length = sizeof(captured->json) - 1;
  }
  memcpy(captured->json, json, copy_length);
  captured->json[copy_length] = '\0';
  captured->length = copy_length;
}

int main(void) {
  if (rc_abi_version() != 1) {
    fprintf(stderr, "unexpected ABI version: %u\n", rc_abi_version());
    return 1;
  }

  if (rc_runtime_send(NULL, (const uint8_t *)"{}", 2) != 1) {
    fprintf(stderr, "null runtime send did not return status 1\n");
    return 1;
  }
  if (rc_runtime_cancel(NULL, 42) != 1) {
    fprintf(stderr, "null runtime cancel did not return status 1\n");
    return 1;
  }
  rc_runtime_destroy(NULL);

  struct captured_event captured = {0};
  rc_runtime_t *runtime = NULL;
  const char *config = "{}";
  int32_t code = rc_runtime_create((const uint8_t *)config, strlen(config), capture_event,
                                   &captured, &runtime);
  if (code != 0 || runtime == NULL) {
    fprintf(stderr, "rc_runtime_create failed: %d\n", code);
    return 1;
  }

  const char *malformed = "{";
  code = rc_runtime_send(runtime, (const uint8_t *)malformed, strlen(malformed));
  if (code != 3) {
    fprintf(stderr, "malformed send returned %d, expected 3\n", code);
    rc_runtime_destroy(runtime);
    return 1;
  }

  const char *command =
      "{\"protocolVersion\":1,\"requestId\":42,\"method\":\"core.ping\",\"params\":{}}";
  code = rc_runtime_send(runtime, (const uint8_t *)command, strlen(command));
  if (code != 0) {
    fprintf(stderr, "rc_runtime_send failed: %d\n", code);
    rc_runtime_destroy(runtime);
    return 1;
  }

  for (int i = 0; i < 100 && captured.length == 0; i++) {
    usleep(10000);
  }

  rc_runtime_destroy(runtime);

  if (captured.length == 0) {
    fprintf(stderr, "no event captured\n");
    return 1;
  }
  if (strstr(captured.json, "\"requestId\":42") == NULL ||
      strstr(captured.json, "\"pong\":true") == NULL) {
    fprintf(stderr, "unexpected event: %s\n", captured.json);
    return 1;
  }

  puts(captured.json);
  return 0;
}
