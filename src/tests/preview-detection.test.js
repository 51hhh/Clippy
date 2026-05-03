import { describe, it, expect, vi } from "vitest";

// Mock Tauri API — preview-panel.js imports from api.js
vi.mock("../js/api.js", () => ({
  getClipImage: vi.fn(),
  getClipDetail: vi.fn(),
  setPreviewVisible: vi.fn(),
  ocrAvailable: vi.fn(),
  ocrImage: vi.fn(),
  getConfig: vi.fn(),
  fetchUrlMeta: vi.fn(),
}));
vi.mock("../i18n/i18n.js", () => ({
  t: (key) => key,
}));

import { __test__ as T } from "../js/preview-panel.js";

// ─── isReadable ─────────────────────────────────────────────

describe("isReadable", () => {
  it("returns true for ASCII text", () => {
    expect(T.isReadable("Hello, world!")).toBe(true);
  });
  it("returns true for Chinese text", () => {
    expect(T.isReadable("你好世界")).toBe(true);
  });
  it("returns true for Latin Extended (French, etc.)", () => {
    expect(T.isReadable("Héllo café résumé")).toBe(true);
  });
  it("returns true for Cyrillic text", () => {
    expect(T.isReadable("Привет мир")).toBe(true);
  });
  it("returns false for empty string", () => {
    expect(T.isReadable("")).toBe(false);
  });
  it("returns false for binary data", () => {
    const bin = String.fromCharCode(0, 1, 2, 3, 4, 5, 6, 7, 8, 14, 15, 16);
    expect(T.isReadable(bin)).toBe(false);
  });
  it("returns true for text with newlines and tabs", () => {
    expect(T.isReadable("line1\nline2\ttab")).toBe(true);
  });
});

// ─── Base64 ─────────────────────────────────────────────────

describe("isBase64", () => {
  it("detects valid Base64 text", () => {
    const b64 = btoa("Hello, World! This is base64");
    const result = T.isBase64(b64);
    expect(result).toBeTruthy();
    expect(result.type).toBe("base64");
    expect(result.decoded).toBe("Hello, World! This is base64");
  });
  it("rejects short strings", () => {
    expect(T.isBase64(btoa("short"))).toBe(false);
  });
  it("rejects non-Base64 characters", () => {
    expect(T.isBase64("This is not base64!!!")).toBe(false);
  });
  it("rejects length not multiple of 4", () => {
    expect(T.isBase64("ABC")).toBe(false);
  });
  it("detects multiline Base64", () => {
    const text = "Hello, World! This is multiline base64 test string!";
    const b64 = btoa(text);
    const multiline = b64.match(/.{1,20}/g).join("\n");
    const result = T.isBase64(multiline);
    expect(result).toBeTruthy();
    expect(result.decoded).toBe(text);
  });
  it("rejects Base64 that decodes to binary", () => {
    // Create Base64 of random binary data (mostly non-printable)
    const binary = String.fromCharCode(...Array.from({ length: 24 }, (_, i) => i));
    const b64 = btoa(binary);
    expect(T.isBase64(b64)).toBe(false);
  });
});

// ─── URL Encoding ───────────────────────────────────────────

describe("isUrlEncoded", () => {
  it("detects URL encoded text", () => {
    const result = T.isUrlEncoded("hello%20world%21");
    expect(result).toBeTruthy();
    expect(result.type).toBe("url-encoded");
    expect(result.decoded).toBe("hello world!");
  });
  it("detects complex URL encoding", () => {
    const result = T.isUrlEncoded("%E4%BD%A0%E5%A5%BD%E4%B8%96%E7%95%8C");
    expect(result).toBeTruthy();
    expect(result.decoded).toBe("你好世界");
  });
  it("rejects text with only one %XX", () => {
    expect(T.isUrlEncoded("hello%20world")).toBe(false);
  });
  it("rejects multiline text", () => {
    expect(T.isUrlEncoded("hello%20\nworld%20")).toBe(false);
  });
  it("rejects already decoded text", () => {
    expect(T.isUrlEncoded("no encoding here")).toBe(false);
  });
});

// ─── HTML Entities ──────────────────────────────────────────

describe("isHtmlEntities", () => {
  it("detects named entities", () => {
    const result = T.isHtmlEntities("&lt;div&gt;hello&lt;/div&gt;");
    expect(result).toBeTruthy();
    expect(result.type).toBe("html-entity");
    expect(result.decoded).toBe("<div>hello</div>");
  });
  it("detects numeric entities", () => {
    const result = T.isHtmlEntities("&#60;p&#62;test&#60;/p&#62;");
    expect(result).toBeTruthy();
    expect(result.decoded).toContain("<p>");
  });
  it("rejects text with less than 2 entities", () => {
    expect(T.isHtmlEntities("only &amp; one")).toBe(false);
  });
  it("rejects plain text", () => {
    expect(T.isHtmlEntities("no entities here")).toBe(false);
  });
});

// ─── Unicode Escape ─────────────────────────────────────────

describe("isUnicodeEscape", () => {
  it("detects unicode escapes", () => {
    const result = T.isUnicodeEscape("\\u4F60\\u597D");
    expect(result).toBeTruthy();
    expect(result.type).toBe("unicode");
    expect(result.decoded).toBe("你好");
  });
  it("rejects text with only one escape", () => {
    expect(T.isUnicodeEscape("hello \\u0041")).toBe(false);
  });
  it("rejects plain text", () => {
    expect(T.isUnicodeEscape("no escapes")).toBe(false);
  });
});

// ─── Hex Encoding ───────────────────────────────────────────

describe("isHexEncoded", () => {
  it("detects hex encoded text", () => {
    // "Hello!" = 48656c6c6f21 (12 chars, but need >= 8 non-hash)
    // "Hello World!" = 48656c6c6f20576f726c6421
    const hex = "48656c6c6f20576f726c6421";
    const result = T.isHexEncoded(hex);
    expect(result).toBeTruthy();
    expect(result.decoded).toBe("Hello World!");
  });
  it("detects 0x prefixed hex", () => {
    const result = T.isHexEncoded("0x48656c6c6f20576f726c6421");
    expect(result).toBeTruthy();
    expect(result.decoded).toBe("Hello World!");
  });
  it("rejects odd-length hex", () => {
    expect(T.isHexEncoded("48656c6")).toBe(false);
  });
  it("excludes hash-length hex (32 chars = MD5)", () => {
    expect(T.isHexEncoded("d41d8cd98f00b204e9800998ecf8427e")).toBe(false);
  });
  it("excludes hash-length hex (64 chars = SHA-256)", () => {
    expect(T.isHexEncoded("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")).toBe(false);
  });
  it("rejects short hex", () => {
    expect(T.isHexEncoded("AABB")).toBe(false);
  });
});

// ─── detectEncoding (integration) ───────────────────────────

describe("detectEncoding", () => {
  it("returns URL encoding result for encoded URL", () => {
    const r = T.detectEncoding("hello%20world%21");
    expect(r.type).toBe("url-encoded");
  });
  it("returns false for plain text", () => {
    expect(T.detectEncoding("just some text")).toBe(false);
  });
  it("prioritizes URL encoding over Base64", () => {
    // This is a URL-encoded string that also passes Base64 length check
    const r = T.detectEncoding("%E4%BD%A0%E5%A5%BD%E4%B8%96%E7%95%8C");
    expect(r.type).toBe("url-encoded");
  });
});

// ─── JWT ────────────────────────────────────────────────────

describe("isJwt", () => {
  const validJwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

  it("detects valid JWT", () => {
    expect(T.isJwt(validJwt)).toBe(true);
  });
  it("rejects non-eyJ prefix", () => {
    expect(T.isJwt("abc.def.ghi")).toBe(false);
  });
  it("rejects two-part token", () => {
    expect(T.isJwt("eyJhbGci.eyJzdWIi")).toBe(false);
  });
  it("rejects four-part token", () => {
    expect(T.isJwt("eyJh.eyJh.eyJh.eyJh")).toBe(false);
  });
});

describe("parseJwt", () => {
  it("parses header and payload", () => {
    const jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    const { header, payload, signature } = T.parseJwt(jwt);
    expect(header).toEqual({ alg: "HS256", typ: "JWT" });
    expect(payload.sub).toBe("1234567890");
    expect(payload.name).toBe("John Doe");
    expect(signature).toBe("SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c");
  });
});

// ─── Hash Identification ────────────────────────────────────

describe("identifyHash", () => {
  it("identifies MD5", () => {
    expect(T.identifyHash("d41d8cd98f00b204e9800998ecf8427e")).toBe("MD5");
  });
  it("identifies SHA-1", () => {
    expect(T.identifyHash("da39a3ee5e6b4b0d3255bfef95601890afd80709")).toBe("SHA-1");
  });
  it("identifies SHA-256", () => {
    expect(T.identifyHash("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")).toBe("SHA-256");
  });
  it("identifies bcrypt", () => {
    expect(T.identifyHash("$2b$12$LJ3m4ys3Lk0TSwMvvZVXwORSvCastLRISTFYr.FKGE3rHoD1eBQky")).toBe("bcrypt");
  });
  it("identifies Argon2", () => {
    expect(T.identifyHash("$argon2id$v=19$m=16384,t=2,p=1$abc")).toBe("Argon2");
  });
  it("returns null for non-hash", () => {
    expect(T.identifyHash("hello world")).toBe(null);
  });
  it("returns null for wrong-length hex", () => {
    expect(T.identifyHash("abcdef1234567890abcdef")).toBe(null);
  });
});

// ─── Encrypted Detection ────────────────────────────────────

describe("detectEncrypted", () => {
  it("detects PGP message", () => {
    expect(T.detectEncrypted("-----BEGIN PGP MESSAGE-----\ndata\n-----END PGP MESSAGE-----")).toBe("PGP");
  });
  it("detects encrypted private key", () => {
    expect(T.detectEncrypted("-----BEGIN ENCRYPTED PRIVATE KEY-----\ndata")).toBe("ENCRYPTED-KEY");
  });
  it("detects OpenSSL salted format", () => {
    expect(T.detectEncrypted("U2FsdGVkX19abcdefghijklmnopqrstuvwxyz1234567890AB")).toBe("AES-OpenSSL");
  });
  it("returns null for plain text", () => {
    expect(T.detectEncrypted("just some plain text")).toBe(null);
  });
});

// ─── Color Detection ────────────────────────────────────────

describe("isColor", () => {
  it("detects 3-digit hex", () => {
    expect(T.isColor("#f00")).toBe(true);
  });
  it("detects 6-digit hex", () => {
    expect(T.isColor("#ff0000")).toBe(true);
  });
  it("detects 8-digit hex (with alpha)", () => {
    expect(T.isColor("#ff000080")).toBe(true);
  });
  it("detects rgb()", () => {
    expect(T.isColor("rgb(255, 0, 0)")).toBe(true);
  });
  it("detects rgba()", () => {
    expect(T.isColor("rgba(255, 0, 0, 0.5)")).toBe(true);
  });
  it("detects hsl()", () => {
    expect(T.isColor("hsl(0, 100%, 50%)")).toBe(true);
  });
  it("detects oklch()", () => {
    expect(T.isColor("oklch(0.7 0.15 30)")).toBe(true);
  });
  it("rejects plain text", () => {
    expect(T.isColor("not a color")).toBe(false);
  });
  it("rejects partial hex", () => {
    expect(T.isColor("#fg0000")).toBe(false);
  });
});

// ─── Timestamp Detection ────────────────────────────────────

describe("isTimestamp", () => {
  it("detects 10-digit Unix timestamp (seconds)", () => {
    expect(T.isTimestamp("1700000000")).toBe(true);
  });
  it("detects 13-digit Unix timestamp (milliseconds)", () => {
    expect(T.isTimestamp("1700000000000")).toBe(true);
  });
  it("rejects too old timestamp", () => {
    expect(T.isTimestamp("0000000001")).toBe(false);
  });
  it("rejects too far future", () => {
    expect(T.isTimestamp("9999999999")).toBe(false);
  });
  it("rejects non-numeric", () => {
    expect(T.isTimestamp("17000abc00")).toBe(false);
  });
  it("rejects 9-digit number", () => {
    expect(T.isTimestamp("123456789")).toBe(false);
  });
});

describe("formatTimestamp", () => {
  it("formats 10-digit timestamp", () => {
    const info = T.formatTimestamp("1700000000");
    expect(info.utc).toBe("2023-11-14T22:13:20.000Z");
    expect(info.precision).toBe("s");
  });
  it("formats 13-digit timestamp", () => {
    const info = T.formatTimestamp("1700000000000");
    expect(info.utc).toBe("2023-11-14T22:13:20.000Z");
    expect(info.precision).toBe("ms");
  });
});

// ─── UUID Detection ─────────────────────────────────────────

describe("isUuid", () => {
  it("detects v4 UUID", () => {
    expect(T.isUuid("550e8400-e29b-41d4-a716-446655440000")).toBe(true);
  });
  it("detects uppercase UUID", () => {
    expect(T.isUuid("550E8400-E29B-41D4-A716-446655440000")).toBe(true);
  });
  it("rejects malformed UUID", () => {
    expect(T.isUuid("550e8400-e29b-41d4-a716")).toBe(false);
  });
  it("rejects non-hex characters", () => {
    expect(T.isUuid("550g8400-e29b-41d4-a716-446655440000")).toBe(false);
  });
});

describe("uuidVersion", () => {
  it("identifies v4", () => {
    expect(T.uuidVersion("550e8400-e29b-41d4-a716-446655440000")).toBe("v4");
  });
  it("identifies v1", () => {
    expect(T.uuidVersion("550e8400-e29b-11d4-a716-446655440000")).toBe("v1");
  });
  it("identifies v7", () => {
    expect(T.uuidVersion("018f6b80-e29b-71d4-a716-446655440000")).toBe("v7");
  });
  it("returns null for unknown version", () => {
    expect(T.uuidVersion("550e8400-e29b-a1d4-a716-446655440000")).toBe(null);
  });
});

// ─── IP Address Detection ───────────────────────────────────

describe("isIpAddress", () => {
  it("detects IPv4", () => {
    expect(T.isIpAddress("192.168.1.1")).toBe(true);
  });
  it("detects IPv4 with CIDR", () => {
    expect(T.isIpAddress("10.0.0.0/8")).toBe(true);
  });
  it("rejects invalid IPv4 octets", () => {
    expect(T.isIpAddress("256.1.1.1")).toBe(false);
  });
  it("detects IPv6 loopback", () => {
    expect(T.isIpAddress("::1")).toBe(true);
  });
  it("detects full IPv6", () => {
    expect(T.isIpAddress("2001:0db8:85a3:0000:0000:8a2e:0370:7334")).toBe(true);
  });
  it("rejects plain text", () => {
    expect(T.isIpAddress("not an ip")).toBe(false);
  });
  it("rejects double :: in IPv6", () => {
    expect(T.isIpAddress("::1::2")).toBe(false);
  });
  it("rejects too-short IPv6 without ::", () => {
    expect(T.isIpAddress("1:2")).toBe(false);
  });
});

describe("ipInfo", () => {
  it("identifies private Class A", () => {
    const info = T.ipInfo("10.0.0.1");
    expect(info.version).toBe("IPv4");
    expect(info.type).toBe("Private (Class A)");
  });
  it("identifies private Class C", () => {
    const info = T.ipInfo("192.168.1.1");
    expect(info.type).toBe("Private (Class C)");
  });
  it("identifies loopback", () => {
    const info = T.ipInfo("127.0.0.1");
    expect(info.type).toBe("Loopback");
  });
  it("identifies public IP", () => {
    const info = T.ipInfo("8.8.8.8");
    expect(info.type).toBe("Public");
  });
  it("identifies IPv6", () => {
    const info = T.ipInfo("2001:db8::1");
    expect(info.version).toBe("IPv6");
  });
  it("detects CIDR notation", () => {
    const info = T.ipInfo("10.0.0.0/8");
    expect(info.cidr).toBe(true);
  });
});
