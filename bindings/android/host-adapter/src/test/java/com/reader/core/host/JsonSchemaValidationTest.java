package com.reader.core.host;

import com.networknt.schema.JsonSchema;
import com.networknt.schema.JsonSchemaFactory;
import com.networknt.schema.SchemaLocation;
import com.networknt.schema.ValidationMessage;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Validates the host adapter's encoded output directly against the upstream
 * protocol JSON Schemas ({@code protocol/reader-command.schema.json} and
 * {@code protocol/reader-event.schema.json}) using a draft 2020-12 validator.
 * This is the strongest contract evidence: the wire bytes the adapter emits
 * must satisfy the protocol's own schema, not just match fixtures.
 */
class JsonSchemaValidationTest {

    private static final String SCHEMA_DIR = System.getProperty("reader.protocol.schema.dir");
    private static JsonSchema commandSchema;
    private static JsonSchema eventSchema;

    @BeforeAll
    static void loadSchemas() throws IOException {
        Assumptions.assumeTrue(SCHEMA_DIR != null, "protocol schema dir not configured");
        Path cmdPath = Paths.get(SCHEMA_DIR, "reader-command.schema.json");
        Path evtPath = Paths.get(SCHEMA_DIR, "reader-event.schema.json");
        Assumptions.assumeTrue(Files.exists(cmdPath), "missing command schema");
        Assumptions.assumeTrue(Files.exists(evtPath), "missing event schema");

        JsonSchemaFactory factory = JsonSchemaFactory.getInstance(
                com.networknt.schema.SpecVersion.VersionFlag.V202012);
        com.fasterxml.jackson.databind.ObjectMapper mapper =
                com.fasterxml.jackson.databind.json.JsonMapper.builder().build();
        // Load the command schema with the protocol dir as the base URI so the
        // cross-schema $ref (reader-event.schema.json#/$defs/CoreError) resolves
        // from disk.
        String protocolDirUri = Paths.get(SCHEMA_DIR).toAbsolutePath().toUri().toString();
        commandSchema = factory.getSchema(
                SchemaLocation.of(protocolDirUri),
                mapper.readTree(Files.readString(cmdPath, StandardCharsets.UTF_8)));
        try (InputStream in = Files.newInputStream(evtPath)) {
            eventSchema = factory.getSchema(in);
        }
    }

    @Test
    void encodeCompleteValidatesAgainstCommandSchema() {
        String cmd = HostReplyCodec.encodeComplete(302L, 1L,
                "{\"status\":\"ok\",\"source\":\"conformance\"}");
        assertValid(commandSchema, cmd, "host.complete");
    }

    @Test
    void encodeErrorValidatesAgainstCommandSchema() {
        String cmd = HostReplyCodec.encodeError(303L, 1L,
                "INTERNAL", "host conformance failure", true);
        assertValid(commandSchema, cmd, "host.error");
    }

    @Test
    void encodeHttpCompleteWithMetadataValidatesAgainstCommandSchema() {
        String result = "{\"status\":200,"
                + "\"headers\":{\"content-type\":\"application/json\"},"
                + "\"body\":\"{\\\"books\\\":[]}\"}";
        String cmd = HostReplyCodec.encodeComplete(502L, 1L, result);
        assertValid(commandSchema, cmd, "host.complete http");
    }

    @Test
    void commanderEncodeCommandValidatesAgainstCommandSchema() {
        String cmd = HostCommander.encodeCommand(42L, "runtime.ping", "{}");
        assertValid(commandSchema, cmd, "runtime.ping");
    }

    @Test
    void hostRequestEventShapeValidatesAgainstEventSchema() {
        // A host.request event the adapter parses — validate it against the
        // event schema (HostRequestEvent def via oneOf).
        String event = "{\"protocolVersion\":1,\"requestId\":301,\"type\":\"host.request\","
                + "\"operationId\":1,\"capability\":\"host.smoke.echo\","
                + "\"params\":{\"message\":\"hi\"}}";
        assertValid(eventSchema, event, "host.request event");
    }

    private static void assertValid(JsonSchema schema, String json, String label) {
        com.fasterxml.jackson.databind.JsonNode node;
        try {
            node = com.fasterxml.jackson.databind.json.JsonMapper.builder().build().readTree(json);
        } catch (com.fasterxml.jackson.core.JsonProcessingException e) {
            throw new AssertionError(label + ": failed to parse JSON: " + e.getMessage(), e);
        }
        Set<ValidationMessage> errors = schema.validate(node);
        assertTrue(errors.isEmpty(),
                label + " failed schema validation: " + errors);
    }
}
