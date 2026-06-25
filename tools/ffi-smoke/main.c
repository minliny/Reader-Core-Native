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

_Static_assert(RC_CREATE_PANIC == -1, "RC_CREATE_PANIC changed");
_Static_assert(RC_CREATE_OK == 0, "RC_CREATE_OK changed");
_Static_assert(RC_CREATE_NULL_OUT_RUNTIME == 2,
               "RC_CREATE_NULL_OUT_RUNTIME changed");
_Static_assert(RC_CREATE_NULL_CALLBACK == 3,
               "RC_CREATE_NULL_CALLBACK changed");
_Static_assert(RC_CREATE_INVALID_CONFIG == 4,
               "RC_CREATE_INVALID_CONFIG changed");
_Static_assert(RC_SEND_PANIC == -1, "RC_SEND_PANIC changed");
_Static_assert(RC_SEND_OK == 0, "RC_SEND_OK changed");
_Static_assert(RC_SEND_NULL_RUNTIME == 1, "RC_SEND_NULL_RUNTIME changed");
_Static_assert(RC_SEND_NULL_COMMAND == 2, "RC_SEND_NULL_COMMAND changed");
_Static_assert(RC_SEND_INVALID_COMMAND == 3,
               "RC_SEND_INVALID_COMMAND changed");
_Static_assert(RC_SEND_PROTOCOL_ERROR == 4,
               "RC_SEND_PROTOCOL_ERROR changed");
_Static_assert(RC_CANCEL_PANIC == -1, "RC_CANCEL_PANIC changed");
_Static_assert(RC_CANCEL_OK == 0, "RC_CANCEL_OK changed");
_Static_assert(RC_CANCEL_NULL_RUNTIME == 1,
               "RC_CANCEL_NULL_RUNTIME changed");
_Static_assert(RC_OK == 0, "RC_OK changed");
_Static_assert(RC_ERR_UNKNOWN_METHOD == 1, "RC_ERR_UNKNOWN_METHOD changed");
_Static_assert(RC_ERR_INVALID_PARAMS == 2, "RC_ERR_INVALID_PARAMS changed");
_Static_assert(RC_ERR_INVALID_PROTOCOL_VERSION == 3,
               "RC_ERR_INVALID_PROTOCOL_VERSION changed");
_Static_assert(RC_ERR_CANCELLED == 4, "RC_ERR_CANCELLED changed");
_Static_assert(RC_ERR_INVALID_MESSAGE == 5, "RC_ERR_INVALID_MESSAGE changed");
_Static_assert(RC_ERR_INTERNAL == 6, "RC_ERR_INTERNAL changed");

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
  char msg[MSG_BUF];
  int32_t code = RC_OK;

  // --- Failure paths that need no runtime -------------------------------
  if (rc_runtime_send(NULL, (const uint8_t *)"{}", 2) !=
      RC_SEND_NULL_RUNTIME) {
    return fail("null runtime send did not return RC_SEND_NULL_RUNTIME");
  }
  code = rc_last_error(NULL, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE) {
    fprintf(stderr, "null-buffer last_error code=%d\n", code);
    return fail("null-buffer last_error did not return INVALID_MESSAGE");
  }
  strcpy(msg, "stale");
  code = rc_last_error(msg, 0);
  if (code != RC_ERR_INVALID_MESSAGE || strcmp(msg, "stale") != 0) {
    fprintf(stderr, "zero-cap last_error: code=%d msg=%s\n", code, msg);
    return fail("zero-cap last_error wrote message or changed code");
  }
  (void)rc_abi_version();
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "runtime handle")) {
    fprintf(stderr, "abi version last_error side effect: code=%d msg=%s\n", code,
            msg);
    return fail("rc_abi_version touched last_error");
  }
  if (rc_runtime_cancel(NULL, 42) != RC_CANCEL_NULL_RUNTIME) {
    return fail("null runtime cancel did not return RC_CANCEL_NULL_RUNTIME");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "runtime handle")) {
    fprintf(stderr, "null cancel last_error: code=%d msg=%s\n", code, msg);
    return fail("null runtime cancel did not record INVALID_MESSAGE");
  }
  rc_runtime_destroy(NULL); // no-op contract
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("null destroy did not clear last_error");
  }

  // --- Create rejection paths -------------------------------------------
  if (rc_runtime_create(NULL, 0, capture_event, NULL, NULL) !=
      RC_CREATE_NULL_OUT_RUNTIME) {
    return fail("null out_runtime did not return RC_CREATE_NULL_OUT_RUNTIME");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "out_runtime")) {
    fprintf(stderr, "null out_runtime last_error: code=%d msg=%s\n", code, msg);
    return fail("null out_runtime did not record INVALID_MESSAGE");
  }
  rc_runtime_t *no_runtime = NULL;
  rc_runtime_t *sentinel = (rc_runtime_t *)(uintptr_t)1;
  if (rc_runtime_create(NULL, 0, NULL, NULL, &no_runtime) !=
          RC_CREATE_NULL_CALLBACK ||
      no_runtime != NULL) {
    return fail("null callback did not return RC_CREATE_NULL_CALLBACK");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "event callback")) {
    fprintf(stderr, "null callback last_error: code=%d msg=%s\n", code, msg);
    return fail("null callback did not record INVALID_MESSAGE");
  }
  if (rc_runtime_create(NULL, 0, NULL, NULL, &sentinel) !=
          RC_CREATE_NULL_CALLBACK ||
      sentinel != NULL) {
    return fail("create failure did not clear out_runtime");
  }

  // Invalid config -> RC_CREATE_INVALID_CONFIG + structured INVALID_MESSAGE.
  sentinel = (rc_runtime_t *)(uintptr_t)1;
  if (rc_runtime_create(NULL, 1, capture_event, NULL, &sentinel) !=
          RC_CREATE_INVALID_CONFIG ||
      sentinel != NULL) {
    return fail("null config with non-zero length did not return RC_CREATE_INVALID_CONFIG");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "config_json")) {
    fprintf(stderr, "null config last_error: code=%d msg=%s\n", code, msg);
    return fail("null config did not record INVALID_MESSAGE");
  }

  const char *bad_config = "{not json";
  sentinel = (rc_runtime_t *)(uintptr_t)1;
  if (rc_runtime_create((const uint8_t *)bad_config, strlen(bad_config),
                        capture_event, NULL, &sentinel) !=
          RC_CREATE_INVALID_CONFIG ||
      sentinel != NULL) {
    return fail("invalid config did not return RC_CREATE_INVALID_CONFIG");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || msg[0] == '\0') {
    fprintf(stderr, "invalid config last_error: code=%d msg=%s\n", code, msg);
    return fail("invalid config did not record INVALID_MESSAGE");
  }

  struct channel defaults_ch;
  memset(&defaults_ch, 0, sizeof defaults_ch);
  pthread_mutex_init(&defaults_ch.mutex, NULL);
  rc_runtime_t *defaults_rt = NULL;
  if (rc_runtime_create(NULL, 0, capture_event, &defaults_ch, &defaults_rt) !=
          RC_CREATE_OK ||
      defaults_rt == NULL) {
    return fail("null config with zero length did not create defaults runtime");
  }
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("default create did not clear last_error");
  }
  rc_runtime_destroy(defaults_rt);
  pthread_mutex_destroy(&defaults_ch.mutex);

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
  if (code != RC_CREATE_OK || rt == NULL) {
    fprintf(stderr, "rc_runtime_create failed: %d\n", code);
    return 1;
  }
  // A successful create clears the last-error slot and the message buffer.
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("successful create did not clear last_error");
  }

  // --- Synchronous send failures (no events emitted) --------------------
  if (rc_runtime_send(rt, NULL, 1) != RC_SEND_NULL_COMMAND) {
    return fail("null command with non-zero length did not return RC_SEND_NULL_COMMAND");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command_json")) {
    fprintf(stderr, "null command last_error: code=%d msg=%s\n", code, msg);
    return fail("null command did not record INVALID_MESSAGE");
  }
  if (rc_runtime_cancel(rt, 123456) != RC_CANCEL_OK) {
    return fail("cancel missing request did not return RC_CANCEL_OK");
  }
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("cancel missing request did not clear last_error");
  }

  if (rc_runtime_send(rt, NULL, 0) != RC_SEND_INVALID_COMMAND) {
    return fail("zero-length command did not return RC_SEND_INVALID_COMMAND");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    fprintf(stderr, "zero-length command last_error: code=%d msg=%s\n", code,
            msg);
    return fail("zero-length command did not record INVALID_MESSAGE");
  }

  if (rc_runtime_send(rt, (const uint8_t *)"", 0) != RC_SEND_INVALID_COMMAND) {
    return fail("empty command did not return RC_SEND_INVALID_COMMAND");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    fprintf(stderr, "empty command last_error: code=%d msg=%s\n", code, msg);
    return fail("empty command did not record INVALID_MESSAGE");
  }

  if (send_str(rt, "{") != RC_SEND_INVALID_COMMAND) {
    return fail("malformed send did not return RC_SEND_INVALID_COMMAND");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "command JSON")) {
    fprintf(stderr, "malformed last_error: code=%d msg=%s\n", code, msg);
    return fail("malformed send did not record INVALID_MESSAGE");
  }

  const char *params_not_object =
      "{\"protocolVersion\":1,\"requestId\":8,\"method\":\"runtime.ping\","
      "\"params\":[]}";
  if (send_str(rt, params_not_object) != RC_SEND_INVALID_COMMAND) {
    return fail("non-object params did not return RC_SEND_INVALID_COMMAND");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_PARAMS || !contains(msg, "params")) {
    fprintf(stderr, "non-object params last_error: code=%d msg=%s\n", code,
            msg);
    return fail("non-object params did not record INVALID_PARAMS");
  }
  if (channel_count(&ch) != 0) {
    return fail("non-object params emitted an async event");
  }

  const char *proto_v2 =
      "{\"protocolVersion\":2,\"requestId\":9,\"method\":\"runtime.ping\","
      "\"params\":{}}";
  if (send_str(rt, proto_v2) != RC_SEND_PROTOCOL_ERROR) {
    return fail("protocol v2 send did not return RC_SEND_PROTOCOL_ERROR");
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
               "info\",\"params\":{}}") != RC_SEND_OK) {
    return fail("core.info send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no core.info event");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"requestId\":10") || !contains(event, "capabilities") ||
      !contains(event, "runtime.ping")) {
    fprintf(stderr, "unexpected core.info event: %s\n", event);
    return fail("core.info event shape");
  }

  // --- runtime.ping + last_error cleared on success ---------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":11,\"method\":\"runtime."
               "ping\",\"params\":{}}") != RC_SEND_OK) {
    return fail("runtime.ping send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no ping event");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"requestId\":11") || !contains(event, "\"pong\":true")) {
    fprintf(stderr, "unexpected ping event: %s\n", event);
    return fail("ping event shape");
  }
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("successful send did not clear last_error");
  }

  // --- Duplicate active requestId (first must stay pending) -------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":50,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{}}}") != RC_SEND_OK) {
    return fail("hostSmoke(50) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 50");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":50")) {
    return fail("host.request(50) shape");
  }
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":50,\"method\":\"runtime."
               "ping\",\"params\":{}}") != RC_SEND_PROTOCOL_ERROR) {
    return fail("duplicate requestId did not return RC_SEND_PROTOCOL_ERROR");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INVALID_MESSAGE || !contains(msg, "duplicate")) {
    fprintf(stderr, "duplicate last_error: code=%d msg=%s\n", code, msg);
    return fail("duplicate did not record INVALID_MESSAGE");
  }
  if (rc_runtime_cancel(rt, 50) != RC_CANCEL_OK) {
    return fail("cancel(50) failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no cancelled event for 50");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"requestId\":50") ||
      !contains(event, "\"CANCELLED\"")) {
    return fail("cancelled(50) shape");
  }

  // --- host.request -> host.complete round trip -------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":60,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{\"hello\":\"world\"}}}") != RC_SEND_OK) {
    return fail("hostSmoke(60) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 60");
  }
  ev++;
  uint64_t op60 = 0;
  if (!json_u64(event, "operationId", &op60) ||
      !contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"capability\":\"host.smoke.echo\"") ||
      !contains(event, "\"hello\":\"world\"")) {
    fprintf(stderr, "host.request(60): %s\n", event);
    return fail("host.request(60) shape");
  }

  char complete[256];
  snprintf(complete, sizeof complete,
           "{\"protocolVersion\":1,\"requestId\":61,\"method\":\"host."
           "complete\",\"params\":{\"operationId\":%llu,\"result\":{\"echoed\":"
           "true}}}",
           (unsigned long long)op60);
  if (send_str(rt, complete) != RC_SEND_OK) {
    return fail("host.complete(60) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no result event for 60");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":60") ||
      !contains(event, "\"echoed\":true")) {
    fprintf(stderr, "result(60): %s\n", event);
    return fail("host.complete result shape");
  }

  // --- host.request -> host.error ---------------------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":62,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{}}}") != RC_SEND_OK) {
    return fail("hostSmoke(62) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 62");
  }
  ev++;
  uint64_t op62 = 0;
  if (!json_u64(event, "operationId", &op62) ||
      !contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":62")) {
    return fail("host.request(62) shape");
  }
  char err_cmd[320];
  snprintf(err_cmd, sizeof err_cmd,
           "{\"protocolVersion\":1,\"requestId\":63,\"method\":\"host."
           "error\",\"params\":{\"operationId\":%llu,\"error\":{\"code\":"
           "\"INTERNAL\",\"message\":\"host failed\",\"retryable\":true}}}",
           (unsigned long long)op62);
  if (send_str(rt, err_cmd) != RC_SEND_OK) {
    return fail("host.error(62) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no error event for 62");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":62") ||
      !contains(event, "\"INTERNAL\"") || !contains(event, "\"retryable\":true")) {
    fprintf(stderr, "error(62): %s\n", event);
    return fail("host.error result shape");
  }

  // --- host.complete for an unknown operation -> async INVALID_PARAMS ----
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":66,\"method\":\"host."
               "complete\",\"params\":{\"operationId\":999999,\"result\":{"
               "\"ok\":true}}}") != RC_SEND_OK) {
    return fail("unknown host.complete send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no error event for unknown host.complete");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":66") ||
      !contains(event, "\"INVALID_PARAMS\"")) {
    fprintf(stderr, "unknown host.complete error: %s\n", event);
    return fail("unknown host.complete error shape");
  }
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("unknown host.complete send left synchronous last_error");
  }

  // --- remote http.execute completion carries metadata ------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":68,\"method\":\"book."
               "search\",\"params\":{\"sourceId\":\"ffi-http-src\","
               "\"searchRequest\":{\"url\":\"https://books.example.test/"
               "search?q=abi\",\"headers\":{\"Accept\":\"application/json\"}},"
               "\"source\":{\"sourceId\":\"ffi-http-src\",\"name\":\"FFI HTTP "
               "Source\",\"baseUrl\":\"https://books.example.test\",\"rules\":{"
               "\"search\":[{\"kind\":\"jsonPath\",\"path\":\"$.books[*]\"}]}}"
               "}}") != RC_SEND_OK) {
    return fail("book.search http request send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no http.execute host.request event");
  }
  ev++;
  uint64_t http_op = 0;
  if (!json_u64(event, "operationId", &http_op) ||
      !contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":68") ||
      !contains(event, "\"capability\":\"http.execute\"") ||
      !contains(event, "search?q=abi") ||
      !contains(event, "\"Accept\":\"application/json\"")) {
    fprintf(stderr, "http.execute request: %s\n", event);
    return fail("http.execute request shape");
  }
  char http_complete[512];
  snprintf(http_complete, sizeof http_complete,
           "{\"protocolVersion\":1,\"requestId\":69,\"method\":\"host."
           "complete\",\"params\":{\"operationId\":%llu,\"result\":{\"status\":"
           "200,\"headers\":{\"content-type\":\"application/json\"},\"body\":"
           "\"{\\\"books\\\":[]}\"}}}",
           (unsigned long long)http_op);
  if (send_str(rt, http_complete) != RC_SEND_OK) {
    return fail("http host.complete send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no result event for http completion");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":68") ||
      !contains(event, "\"books\":[]") ||
      !contains(event, "\"http\"") ||
      !contains(event, "\"status\":200") ||
      !contains(event, "\"content-type\":\"application/json\"")) {
    fprintf(stderr, "http completion result: %s\n", event);
    return fail("http completion result shape");
  }
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("http completion left synchronous last_error");
  }

  // --- method-specific invalid params -> async INVALID_PARAMS ------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":67,\"method\":\"reading."
               "progress.update\",\"params\":{\"bookId\":\"1\","
               "\"chapterIndex\":2,\"chapterOffset\":128,"
               "\"chapterProgress\":0.5,\"syncToken\":\"host-owned\"}}") !=
      RC_SEND_OK) {
    return fail("reading.progress.update invalid params send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no error event for invalid reading.progress.update params");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":67") ||
      !contains(event, "\"INVALID_PARAMS\"") ||
      !contains(event, "reading.progress.update")) {
    fprintf(stderr, "invalid reading.progress.update error: %s\n", event);
    return fail("invalid reading.progress.update error shape");
  }
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("invalid reading.progress.update send left synchronous last_error");
  }

  // --- unknown method -> async error event ------------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":64,\"method\":\"no.such."
               "method\",\"params\":{}}") != RC_SEND_OK) {
    return fail("unknown method send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no error event for 64");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"requestId\":64") ||
      !contains(event, "\"UNKNOWN_METHOD\"")) {
    fprintf(stderr, "error(64): %s\n", event);
    return fail("unknown method error shape");
  }

  // --- cancel a pending host.request ------------------------------------
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":65,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{}}}") != RC_SEND_OK) {
    return fail("hostSmoke(65) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 65");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":65")) {
    fprintf(stderr, "host.request(65): %s\n", event);
    return fail("host.request(65) shape");
  }
  if (rc_runtime_cancel(rt, 65) != RC_CANCEL_OK) {
    return fail("cancel(65) failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no cancelled event for 65");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"requestId\":65") ||
      !contains(event, "\"CANCELLED\"")) {
    fprintf(stderr, "cancelled(65): %s\n", event);
    return fail("cancelled(65) shape");
  }

  // --- runtime.shutdown cancels pending work and blocks future sends -----
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":70,\"method\":\"runtime."
               "hostSmoke\",\"params\":{\"capability\":\"host.smoke.echo\","
               "\"params\":{}}}") != RC_SEND_OK) {
    return fail("hostSmoke(70) send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no host.request for 70");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"host.request\"") ||
      !contains(event, "\"requestId\":70")) {
    fprintf(stderr, "host.request(70): %s\n", event);
    return fail("host.request(70) shape");
  }
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":71,\"method\":\"runtime."
               "shutdown\",\"params\":{}}") != RC_SEND_OK) {
    return fail("runtime.shutdown send failed");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no cancelled event for shutdown");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"error\"") ||
      !contains(event, "\"requestId\":70") ||
      !contains(event, "\"CANCELLED\"")) {
    fprintf(stderr, "shutdown cancelled event: %s\n", event);
    return fail("shutdown cancelled event shape");
  }
  if (wait_event(&ch, ev, event, sizeof event) != 0) {
    return fail("no runtime.shutdown result");
  }
  ev++;
  if (!contains(event, "\"protocolVersion\":1") ||
      !contains(event, "\"type\":\"result\"") ||
      !contains(event, "\"requestId\":71") ||
      !contains(event, "\"shuttingDown\":true") ||
      !contains(event, "\"cancelledRequestIds\":[70]")) {
    fprintf(stderr, "shutdown result: %s\n", event);
    return fail("shutdown result shape");
  }
  if (send_str(rt,
               "{\"protocolVersion\":1,\"requestId\":72,\"method\":\"runtime."
               "ping\",\"params\":{}}") != RC_SEND_PROTOCOL_ERROR) {
    return fail("runtime.ping after shutdown did not return RC_SEND_PROTOCOL_ERROR");
  }
  code = rc_last_error(msg, sizeof msg);
  if (code != RC_ERR_INTERNAL || !contains(msg, "shutting down")) {
    fprintf(stderr, "post-shutdown send last_error: code=%d msg=%s\n", code,
            msg);
    return fail("post-shutdown send did not record INTERNAL");
  }
  if (channel_count(&ch) != ev) {
    return fail("post-shutdown send emitted an async event");
  }

  if (rc_runtime_send(rt, (const uint8_t *)"{", 1) != RC_SEND_INVALID_COMMAND) {
    return fail("pre-destroy invalid command did not return RC_SEND_INVALID_COMMAND");
  }
  rc_runtime_destroy(rt);
  strcpy(msg, "stale");
  if (rc_last_error(msg, sizeof msg) != RC_OK || msg[0] != '\0') {
    return fail("successful destroy did not clear last_error");
  }
  pthread_mutex_destroy(&ch.mutex);

  puts("c-abi-smoke: ok");
  return 0;
}
