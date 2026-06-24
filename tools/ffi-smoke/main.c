// C ABI smoke for reader_core.h.
//
// Drives the runtime the way a real C host would: create (with config), send
// commands, receive events via the callback, answer host.request with
// host.complete / host.error, cancel pending requests, and read structured
// errors through rc_last_error. Failure paths are covered explicitly.

#include "reader_core.h"

#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define MAX_EVENTS 32
#define EVENT_BUF 8192
#define MSG_BUF 256

struct captured_event {
  char json[EVENT_BUF];
  size_t length;
};

struct channel {
  pthread_mutex_t mutex;
  struct captured_event events[MAX_EVENTS];
  size_t count;
};

static void capture_event(void *context, const uint8_t *json,
                          size_t json_length) {
  struct channel *ch = (struct channel *)context;
  pthread_mutex_lock(&ch->mutex);
  if (ch->count < MAX_EVENTS) {
    size_t copy = json_length < (EVENT_BUF - 1) ? json_length : (EVENT_BUF - 1);
    memcpy(ch->events[ch->count].json, json, copy);
    ch->events[ch->count].json[copy] = '\0';
    ch->events[ch->count].length = copy;
    ch->count++;
  }
  pthread_mutex_unlock(&ch->mutex);
}

static size_t channel_count(struct channel *ch) {
  pthread_mutex_lock(&ch->mutex);
  size_t n = ch->count;
  pthread_mutex_unlock(&ch->mutex);
  return n;
}

// Wait until at least `index + 1` events have arrived, then copy event[index]
// into `out`. Returns 0 on success, non-zero on timeout.
static int wait_event(struct channel *ch, size_t index, char *out, size_t cap) {
  for (int i = 0; i < 1000; i++) {
    if (channel_count(ch) > index) {
      break;
    }
    usleep(5000);
  }
  pthread_mutex_lock(&ch->mutex);
  if (index >= ch->count) {
    pthread_mutex_unlock(&ch->mutex);
    return 1;
  }
  strncpy(out, ch->events[index].json, cap - 1);
  out[cap - 1] = '\0';
  pthread_mutex_unlock(&ch->mutex);
  return 0;
}

static int send_str(rc_runtime_t *rt, const char *json) {
  return rc_runtime_send(rt, (const uint8_t *)json, strlen(json));
}

// Extract a `"key":<uint64>` value from a JSON blob. Returns 0 on miss.
static int json_u64(const char *json, const char *key, uint64_t *out) {
  char needle[64];
  snprintf(needle, sizeof needle, "\"%s\":", key);
  const char *p = strstr(json, needle);
  if (p == NULL) {
    return 0;
  }
  p += strlen(needle);
  while (*p == ' ' || *p == '\t' || *p == '\n') {
    p++;
  }
  char *end = NULL;
  unsigned long long v = strtoull(p, &end, 10);
  if (end == p) {
    return 0;
  }
  *out = (uint64_t)v;
  return 1;
}

static int contains(const char *haystack, const char *needle) {
  return strstr(haystack, needle) != NULL;
}

static int fail(const char *msg) {
  fprintf(stderr, "FAIL: %s\n", msg);
  return 1;
}

int main(void) {
  if (rc_abi_version() != 1) {
    fprintf(stderr, "unexpected ABI version: %u\n", rc_abi_version());
    return 1;
  }

  // --- Failure paths that need no runtime -------------------------------
  if (rc_runtime_send(NULL, (const uint8_t *)"{}", 2) != 1) {
    return fail("null runtime send did not return status 1");
  }
  if (rc_runtime_cancel(NULL, 42) != 1) {
    return fail("null runtime cancel did not return status 1");
  }
  rc_runtime_destroy(NULL); // no-op contract

  // --- Create rejection paths -------------------------------------------
  if (rc_runtime_create(NULL, 0, capture_event, NULL, NULL) != 2) {
    return fail("null out_runtime did not return status 2");
  }
  rc_runtime_t *no_runtime = NULL;
  if (rc_runtime_create(NULL, 0, NULL, NULL, &no_runtime) != 3 ||
      no_runtime != NULL) {
    return fail("null callback did not return status 3");
  }

  // Invalid config -> status 4 + structured INVALID_MESSAGE.
  const char *bad_config = "{not json";
  if (rc_runtime_create((const uint8_t *)bad_config, strlen(bad_config),
                        capture_event, NULL, &no_runtime) != 4) {
    return fail("invalid config did not return status 4");
  }
  char msg[MSG_BUF];
  int32_t code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || msg[0] == '\0') {
    fprintf(stderr, "invalid config last_error: code=%d msg=%s\n", code, msg);
    return fail("invalid config did not record INVALID_MESSAGE");
  }

  // --- Create a real runtime --------------------------------------------
  struct channel ch;
  memset(&ch, 0, sizeof ch);
  pthread_mutex_init(&ch.mutex, NULL);

  rc_runtime_t *rt = NULL;
  const char *config =
      "{\"dataDirectory\":\"/tmp/reader-smoke-data\",\"cacheDirectory\":\"/tmp/"
      "reader-smoke-cache\"}";
  code = rc_runtime_create((const uint8_t *)config, strlen(config),
                           capture_event, &ch, &rt);
  if (code != 0 || rt == NULL) {
    fprintf(stderr, "rc_runtime_create failed: %d\n", code);
    return 1;
  }
  // A successful create clears the last-error slot.
  if (rc_last_error(msg, sizeof msg) != RC_OK) {
    return fail("successful create did not clear last_error");
  }

  // --- Synchronous send failures (no events emitted) --------------------
  if (send_str(rt, "{") != 3) {
    return fail("malformed send did not return status 3");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    fprintf(stderr, "malformed last_error: code=%d msg=%s\n", code, msg);
    return fail("malformed send did not record INVALID_MESSAGE");
  }

  const char *proto_v2 =
      "{\"protocolVersion\":2,\"requestId\":9,\"method\":\"runtime.ping\","
      "\"params\":{}}";
  if (send_str(rt, proto_v2) != 4) {
    return fail("protocol v2 send did not return status 4");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_PROTOCOL_VERSION ||
      !contains(msg, "protocolVersion")) {
    fprintf(stderr, "proto v2 last_error: code=%d msg=%s\n", code, msg);
    return fail("protocol v2 did not record INVALID_PROTOCOL_VERSION");
  }

  // --- core.info ---------------------------------------------------------
  size_t ev = 0;
  char event[EVENT_BUF];
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":10,\"method\":\"core."
               "info\",\"params\":{}}") != 0) {
    return fail("core.info send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no core.info event");
  }
  ev++;
  if (!contains(event, "\"requestId\":10") || !contains(event, "capabilities") ||
      !contains(event, "runtime.ping")) {
    fprintf(stderr, "unexpected core.info event: %s\n", event);
    return fail("core.info event shape");
  }

  // --- runtime.ping + last_error cleared on success ---------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":11,\"method\":\"runtime."
               "ping\",\"params\":{}}") != 0) {
    return fail("runtime.ping send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no ping event");
  }
  ev++;
  if (!contains(event, "\"requestId\":11") || !contains(event, "\"pong\":true")) {
    fprintf(stderr, "unexpected ping event: %s\n", event);
    return fail("ping event shape");
  }
  if (rc_last_error(msg, sizeof msg) != RC_OK) {
    return fail("successful send did not clear last_error");
  }

  // --- Duplicate active requestId (first must stay pending) -------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":50,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{}}}") != 0) {
    return fail("hostSmoke(50) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 50");
  }
  ev++;
  if (!contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":50")) {
    return fail("host.request(50) shape");
  }
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":50,\"method\":\"runtime."
               "ping\",\"params\":{}}") != 4) {
    return fail("duplicate requestId did not return status 4");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "duplicate")) {
    fprintf(stderr, "duplicate last_error: code=%d msg=%s\n", code, msg);
    return fail("duplicate did not record INVALID_MESSAGE");
  }
  if (rc_runtime_cancel(rt, 50) != 0) {
    return fail("cancel(50) failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no cancelled event for 50");
  }
  ev++;
  if (!contains(event, "\"requestId\":50") ||
      !contains(event, "\"CANCELLED\"")) {
    return fail("cancelled(50) shape");
  }

  // --- host.request -> host.complete round trip -------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":60,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{\"hello\":\"world\"}}}") != 0) {
    return fail("hostSmoke(60) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 60");
  }
  ev++;
  uint64_t op60 = 0;
  if (!json_u64(event, "operationId", &op60) ||
      !contains(event, "\"capability\":\"host.smoke.echo\"")) {
    fprintf(stderr, "host.request(60): %s\n", event);
    return fail("host.request(60) shape");
  }

  char complete[256];
  snprintf(complete, sizeof complete,
           "{\"protocolVersion\":1,\"requestId\":61,\"method\":\"host."
           "complete\",\"params\":{\"operationId\":%llu,\"result\":{\"echoed\":"
           "true}}}",
           (unsigned long long)op60);
  if (send_str(rt, complete) != 0) {
    return fail("host.complete(60) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no result event for 60");
  }
  ev++;
  if (!contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":60") ||
      !contains(event, "\"echoed\":true")) {
    fprintf(stderr, "result(60): %s\n", event);
    return fail("host.complete result shape");
  }

  // --- host.request -> host.error ---------------------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":62,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{}}}") != 0) {
    return fail("hostSmoke(62) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 62");
  }
  ev++;
  uint64_t op62 = 0;
  if (!json_u64(event, "operationId", &op62)) {
    return fail("host.request(62) shape");
  }
  char err_cmd[320];
  snprintf(err_cmd, sizeof err_cmd,
           "{\"protocolVersion\":1,\"requestId\":63,\"method\":\"host."
           "error\",\"params\":{\"operationId\":%llu,\"error\":{\"code\":"
           "\"INTERNAL\",\"message\":\"host failed\",\"retryable\":true}}}",
           (unsigned long long)op62);
  if (send_str(rt, err_cmd) != 0) {
    return fail("host.error(62) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no error event for 62");
  }
  ev++;
  if (!contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":62") ||
      !contains(event, "\"INTERNAL\"") || !contains(event, "\"retryable\":true")) {
    fprintf(stderr, "error(62): %s\n", event);
    return fail("host.error result shape");
  }

  // --- unknown method -> async error event ------------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":64,\"method\":\"no.such."
               "method\",\"params\":{}}") != 0) {
    return fail("unknown method send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no error event for 64");
  }
  ev++;
  if (!contains(event, "\"requestId\":64") ||
      !contains(event, "\"UNKNOWN_METHOD\"")) {
    fprintf(stderr, "error(64): %s\n", event);
    return fail("unknown method error shape");
  }

  // --- cancel a pending host.request ------------------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":65,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{}}}") != 0) {
    return fail("hostSmoke(65) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 65");
  }
  ev++;
  if (rc_runtime_cancel(rt, 65) != 0) {
    return fail("cancel(65) failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no cancelled event for 65");
  }
  ev++;
  if (!contains(event, "\"requestId\":65") ||
      !contains(event, "\"CANCELLED\"")) {
    fprintf(stderr, "cancelled(65): %s\n", event);
    return fail("cancelled(65) shape");
  }

  rc_runtime_destroy(rt);
  pthread_mutex_destroy(&ch.mutex);

  puts("c-abi-smoke: ok");
  return 0;
}
