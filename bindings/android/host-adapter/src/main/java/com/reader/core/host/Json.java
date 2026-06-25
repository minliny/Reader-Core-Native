package com.reader.core.host;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Minimal dependency-free JSON codec for the Android host adapter.
 *
 * <p>The host adapter must parse Core {@code host.request} events and encode
 * {@code host.complete} / {@code host.error} command JSON. Keeping this logic
 * dependency-free lets the adapter run under a plain JVM for Gradle unit tests
 * (no Android framework, no Maven runtime deps) while remaining embeddable in
 * an Android host app. A host app may later swap this for {@code org.json};
 * the adapter's contract does not depend on a particular JSON backend.
 */
public final class Json {

    private Json() {}

    /** Parse a JSON document into a tree of Map / List / String / Number / Boolean / null. */
    public static Object parse(String text) {
        if (text == null) {
            throw new JsonException("null input");
        }
        Parser p = new Parser(text);
        p.skipWs();
        Object v = p.readValue();
        p.skipWs();
        if (p.pos < p.src.length) {
            throw new JsonException("trailing data at " + p.pos);
        }
        return v;
    }

    /** Serialize a value to canonical compact JSON (no incidental whitespace). */
    public static String stringify(Object value) {
        StringBuilder sb = new StringBuilder();
        writeValue(sb, value);
        return sb.toString();
    }

    /** Parse then re-serialize; yields a canonical form for equality comparison. */
    public static String canonicalize(String text) {
        return stringify(parse(text));
    }

    @SuppressWarnings("unchecked")
    private static void writeValue(StringBuilder sb, Object v) {
        if (v == null) {
            sb.append("null");
        } else if (v instanceof String) {
            writeString(sb, (String) v);
        } else if (v instanceof Boolean) {
            sb.append(((Boolean) v) ? "true" : "false");
        } else if (v instanceof Number) {
            sb.append(v.toString());
        } else if (v instanceof Map) {
            sb.append('{');
            boolean first = true;
            for (Map.Entry<String, Object> e : ((Map<String, Object>) v).entrySet()) {
                if (!first) sb.append(',');
                first = false;
                writeString(sb, e.getKey());
                sb.append(':');
                writeValue(sb, e.getValue());
            }
            sb.append('}');
        } else if (v instanceof List) {
            sb.append('[');
            boolean first = true;
            for (Object item : (List<Object>) v) {
                if (!first) sb.append(',');
                first = false;
                writeValue(sb, item);
            }
            sb.append(']');
        } else {
            throw new JsonException("unsupported value type: " + v.getClass());
        }
    }

    static void writeString(StringBuilder sb, String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"': sb.append("\\\""); break;
                case '\\': sb.append("\\\\"); break;
                case '\b': sb.append("\\b"); break;
                case '\f': sb.append("\\f"); break;
                case '\n': sb.append("\\n"); break;
                case '\r': sb.append("\\r"); break;
                case '\t': sb.append("\\t"); break;
                default:
                    if (c < 0x20) {
                        sb.append(String.format("\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
            }
        }
        sb.append('"');
    }

    public static final class JsonException extends RuntimeException {
        public JsonException(String msg) {
            super(msg);
        }
    }

    private static final class Parser {
        final char[] src;
        int pos;

        Parser(String s) {
            this.src = s.toCharArray();
            this.pos = 0;
        }

        void skipWs() {
            while (pos < src.length) {
                char c = src[pos];
                if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
                    pos++;
                } else {
                    break;
                }
            }
        }

        Object readValue() {
            skipWs();
            if (pos >= src.length) {
                throw new JsonException("unexpected end of input");
            }
            char c = src[pos];
            if (c == '{') return readObject();
            if (c == '[') return readArray();
            if (c == '"') return readString();
            if (c == 't' || c == 'f') return readBool();
            if (c == 'n') return readNull();
            if (c == '-' || (c >= '0' && c <= '9')) return readNumber();
            throw new JsonException("unexpected char '" + c + "' at " + pos);
        }

        Map<String, Object> readObject() {
            expect('{');
            LinkedHashMap<String, Object> m = new LinkedHashMap<>();
            skipWs();
            if (peek() == '}') {
                pos++;
                return m;
            }
            while (true) {
                skipWs();
                String key = readString();
                skipWs();
                expect(':');
                Object val = readValue();
                m.put(key, val);
                skipWs();
                char c = next();
                if (c == ',') continue;
                if (c == '}') break;
                throw new JsonException("expected ',' or '}' at " + (pos - 1));
            }
            return m;
        }

        List<Object> readArray() {
            expect('[');
            List<Object> list = new ArrayList<>();
            skipWs();
            if (peek() == ']') {
                pos++;
                return list;
            }
            while (true) {
                list.add(readValue());
                skipWs();
                char c = next();
                if (c == ',') continue;
                if (c == ']') break;
                throw new JsonException("expected ',' or ']' at " + (pos - 1));
            }
            return list;
        }

        String readString() {
            expect('"');
            StringBuilder sb = new StringBuilder();
            while (pos < src.length) {
                char c = src[pos++];
                if (c == '"') {
                    return sb.toString();
                }
                if (c == '\\') {
                    if (pos >= src.length) {
                        throw new JsonException("bad escape");
                    }
                    char e = src[pos++];
                    switch (e) {
                        case '"': sb.append('"'); break;
                        case '\\': sb.append('\\'); break;
                        case '/': sb.append('/'); break;
                        case 'b': sb.append('\b'); break;
                        case 'f': sb.append('\f'); break;
                        case 'n': sb.append('\n'); break;
                        case 'r': sb.append('\r'); break;
                        case 't': sb.append('\t'); break;
                        case 'u':
                            if (pos + 4 > src.length) {
                                throw new JsonException("bad unicode escape");
                            }
                            String hex = new String(src, pos, 4);
                            sb.append((char) Integer.parseInt(hex, 16));
                            pos += 4;
                            break;
                        default:
                            throw new JsonException("bad escape '\\" + e + "'");
                    }
                } else {
                    sb.append(c);
                }
            }
            throw new JsonException("unterminated string");
        }

        Boolean readBool() {
            if (match("true")) return Boolean.TRUE;
            if (match("false")) return Boolean.FALSE;
            throw new JsonException("bad literal at " + pos);
        }

        Object readNull() {
            if (match("null")) return null;
            throw new JsonException("bad literal at " + pos);
        }

        boolean match(String lit) {
            if (pos + lit.length() > src.length) return false;
            for (int i = 0; i < lit.length(); i++) {
                if (src[pos + i] != lit.charAt(i)) return false;
            }
            pos += lit.length();
            return true;
        }

        Number readNumber() {
            int start = pos;
            if (peek() == '-') pos++;
            while (pos < src.length) {
                char c = src[pos];
                if ((c >= '0' && c <= '9') || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-') {
                    pos++;
                } else {
                    break;
                }
            }
            String num = new String(src, start, pos - start);
            if (num.contains(".") || num.contains("e") || num.contains("E")) {
                return Double.parseDouble(num);
            }
            return Long.parseLong(num);
        }

        char peek() {
            return pos < src.length ? src[pos] : '\0';
        }

        char next() {
            if (pos >= src.length) throw new JsonException("unexpected end of input");
            return src[pos++];
        }

        void expect(char c) {
            char a = next();
            if (a != c) {
                throw new JsonException("expected '" + c + "' got '" + a + "' at " + (pos - 1));
            }
        }
    }
}
