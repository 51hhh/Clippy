import { describe, it, expect } from "vitest";
import * as T from "../js/preview/detectors.js";

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
  it("decodes entities without creating user supplied elements", () => {
    const result = T.isHtmlEntities('<img src=x onerror="alert(1)">&amp;&lt;');
    expect(result.decoded).toBe('<img src=x onerror="alert(1)">&<');
    expect(document.querySelector("img")).toBeNull();
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

// ── Email ──────────────────────────────────────────────────
describe("isEmail", () => {
  it("detects simple email", () => {
    expect(T.isEmail("user@example.com")).toBe(true);
  });
  it("detects email with dots and plus", () => {
    expect(T.isEmail("first.last+tag@sub.domain.org")).toBe(true);
  });
  it("rejects missing @", () => {
    expect(T.isEmail("userexample.com")).toBe(false);
  });
  it("rejects multiline", () => {
    expect(T.isEmail("user@example.com\nother")).toBe(false);
  });
  it("rejects too short", () => {
    expect(T.isEmail("a@b")).toBe(false);
  });
  it("rejects space", () => {
    expect(T.isEmail("user @example.com")).toBe(false);
  });
});

describe("emailInfo", () => {
  it("splits local and domain", () => {
    const info = T.emailInfo("user@example.com");
    expect(info.local).toBe("user");
    expect(info.domain).toBe("example.com");
  });
});

// ── MAC Address ────────────────────────────────────────────
describe("isMacAddress", () => {
  it("detects colon-separated MAC", () => {
    expect(T.isMacAddress("AA:BB:CC:DD:EE:FF")).toBe(true);
  });
  it("detects dash-separated MAC", () => {
    expect(T.isMacAddress("aa-bb-cc-dd-ee-ff")).toBe(true);
  });
  it("detects Cisco format", () => {
    expect(T.isMacAddress("aabb.ccdd.eeff")).toBe(true);
  });
  it("detects EUI-64", () => {
    expect(T.isMacAddress("AA:BB:CC:DD:EE:FF:00:11")).toBe(true);
  });
  it("rejects too short", () => {
    expect(T.isMacAddress("AA:BB:CC")).toBe(false);
  });
  it("rejects plain text", () => {
    expect(T.isMacAddress("not a mac")).toBe(false);
  });
});

describe("macInfo", () => {
  it("detects EUI-48 format", () => {
    const info = T.macInfo("aa:bb:cc:dd:ee:ff");
    expect(info.format).toBe("EUI-48");
    expect(info.oui).toBe("AA:BB:CC");
  });
  it("detects Cisco format", () => {
    const info = T.macInfo("aabb.ccdd.eeff");
    expect(info.format).toBe("Cisco");
  });
  it("detects multicast", () => {
    const info = T.macInfo("01:00:5e:00:00:01");
    expect(info.multicast).toBe(true);
  });
  it("detects locally administered", () => {
    const info = T.macInfo("02:00:00:00:00:00");
    expect(info.localAdmin).toBe(true);
  });
  it("detects universally administered unicast", () => {
    const info = T.macInfo("00:1A:2B:3C:4D:5E");
    expect(info.multicast).toBe(false);
    expect(info.localAdmin).toBe(false);
  });
});

// ── Cron ───────────────────────────────────────────────────
describe("isCron", () => {
  it("detects 5-field cron", () => {
    expect(T.isCron("* * * * *")).toBe(true);
  });
  it("detects 6-field cron with seconds", () => {
    expect(T.isCron("0 */5 * * * *")).toBe(true);
  });
  it("detects specific schedule", () => {
    expect(T.isCron("30 8 * * 1-5")).toBe(true);
  });
  it("detects ranges and lists", () => {
    expect(T.isCron("0,30 9-17 * * 1,2,3")).toBe(true);
  });
  it("detects step values", () => {
    expect(T.isCron("*/15 * * * *")).toBe(true);
  });
  it("rejects 4-field", () => {
    expect(T.isCron("* * * *")).toBe(false);
  });
  it("rejects 7-field", () => {
    expect(T.isCron("* * * * * * *")).toBe(false);
  });
  it("rejects plain text", () => {
    expect(T.isCron("not a cron")).toBe(false);
  });
});

describe("cronDescribe", () => {
  it("describes every-minute cron", () => {
    const info = T.cronDescribe("* * * * *");
    expect(info.fields).toBe(5);
    expect(info.description).toBe("Every minute");
  });
  it("describes specific schedule", () => {
    const info = T.cronDescribe("30 8 * * 1-5");
    expect(info.fields).toBe(5);
    expect(info.description).toContain("minute");
    expect(info.description).toContain("hour");
  });
  it("handles 6-field", () => {
    const info = T.cronDescribe("0 */5 * * * *");
    expect(info.fields).toBe(6);
  });
});

// ── Date String ────────────────────────────────────────────
describe("isDateString", () => {
  it("detects ISO 8601 date only", () => {
    expect(T.isDateString("2024-01-15")).toBe(true);
  });
  it("detects ISO 8601 with time", () => {
    expect(T.isDateString("2024-01-15T10:30:00Z")).toBe(true);
  });
  it("detects ISO 8601 with timezone offset", () => {
    expect(T.isDateString("2024-01-15T10:30:00+08:00")).toBe(true);
  });
  it("detects common date format", () => {
    expect(T.isDateString("2024/01/15")).toBe(true);
  });
  it("detects date with time", () => {
    expect(T.isDateString("2024-01-15 10:30:00")).toBe(true);
  });
  it("rejects plain text", () => {
    expect(T.isDateString("not a date")).toBe(false);
  });
  it("rejects too short", () => {
    expect(T.isDateString("2024")).toBe(false);
  });
  it("rejects invalid date", () => {
    expect(T.isDateString("2024-13-45")).toBe(false);
  });
});

describe("dateInfo", () => {
  it("returns parsed date info", () => {
    const info = T.dateInfo("2024-01-15T10:30:00Z");
    expect(info.timestamp).toBe(1705314600);
    expect(info.iso).toBe("2024-01-15T10:30:00.000Z");
    expect(info.relative).toBeDefined();
  });
});

// ── Semantic Version ───────────────────────────────────────
describe("isSemver", () => {
  it("detects simple version", () => {
    expect(T.isSemver("1.2.3")).toBe(true);
  });
  it("detects version with v prefix", () => {
    expect(T.isSemver("v1.2.3")).toBe(true);
  });
  it("detects pre-release", () => {
    expect(T.isSemver("1.0.0-alpha.1")).toBe(true);
  });
  it("detects build metadata", () => {
    expect(T.isSemver("1.0.0+build.123")).toBe(true);
  });
  it("detects full version", () => {
    expect(T.isSemver("v2.1.0-beta.3+sha.abc")).toBe(true);
  });
  it("rejects incomplete version", () => {
    expect(T.isSemver("1.2")).toBe(false);
  });
  it("rejects plain text", () => {
    expect(T.isSemver("hello")).toBe(false);
  });
});

describe("semverInfo", () => {
  it("parses components", () => {
    const info = T.semverInfo("v2.1.3-beta.1+build.42");
    expect(info.major).toBe(2);
    expect(info.minor).toBe(1);
    expect(info.patch).toBe(3);
    expect(info.preRelease).toBe("beta.1");
    expect(info.build).toBe("build.42");
  });
  it("normalizes without prefix", () => {
    const info = T.semverInfo("v1.0.0");
    expect(info.normalized).toBe("1.0.0");
  });
});

// ── Number Base ────────────────────────────────────────────
describe("isNumberBase", () => {
  it("detects hex", () => {
    expect(T.isNumberBase("0xFF")).toBe(true);
  });
  it("detects binary", () => {
    expect(T.isNumberBase("0b1010")).toBe(true);
  });
  it("detects octal", () => {
    expect(T.isNumberBase("0o777")).toBe(true);
  });
  it("rejects plain decimal", () => {
    expect(T.isNumberBase("255")).toBe(false);
  });
  it("rejects text", () => {
    expect(T.isNumberBase("hello")).toBe(false);
  });
});

describe("numberBaseInfo", () => {
  it("converts hex to all bases", () => {
    const info = T.numberBaseInfo("0xFF");
    expect(info.decimal).toBe(255);
    expect(info.binary).toBe("0b11111111");
    expect(info.octal).toBe("0o377");
  });
  it("converts binary to all bases", () => {
    const info = T.numberBaseInfo("0b1010");
    expect(info.decimal).toBe(10);
    expect(info.hex).toBe("0xA");
  });
  it("converts octal to all bases", () => {
    const info = T.numberBaseInfo("0o777");
    expect(info.decimal).toBe(511);
  });
});

// ── CSS Gradient ───────────────────────────────────────────
describe("isGradient", () => {
  it("detects linear gradient", () => {
    expect(T.isGradient("linear-gradient(to right, red, blue)")).toBe(true);
  });
  it("detects radial gradient", () => {
    expect(T.isGradient("radial-gradient(circle, #fff, #000)")).toBe(true);
  });
  it("detects conic gradient", () => {
    expect(T.isGradient("conic-gradient(from 0deg, red, blue)")).toBe(true);
  });
  it("rejects plain text", () => {
    expect(T.isGradient("not a gradient")).toBe(false);
  });
  it("rejects incomplete", () => {
    expect(T.isGradient("linear-gradient")).toBe(false);
  });
});

// ── Data Size ──────────────────────────────────────────────
describe("isDataSize", () => {
  it("detects KB", () => {
    expect(T.isDataSize("1024 KB")).toBe(true);
  });
  it("detects MB with decimal", () => {
    expect(T.isDataSize("1.5 MB")).toBe(true);
  });
  it("detects GiB (binary)", () => {
    expect(T.isDataSize("2 GiB")).toBe(true);
  });
  it("detects bytes", () => {
    expect(T.isDataSize("512 bytes")).toBe(true);
  });
  it("rejects plain text", () => {
    expect(T.isDataSize("hello")).toBe(false);
  });
  it("rejects just number", () => {
    expect(T.isDataSize("1024")).toBe(false);
  });
});

describe("dataSizeInfo", () => {
  it("converts 1 MB to bytes", () => {
    const info = T.dataSizeInfo("1 MB");
    expect(info.bytes).toBe(1e6);
    expect(info.conversions.length).toBeGreaterThan(1);
  });
  it("converts 1 GiB to bytes", () => {
    const info = T.dataSizeInfo("1 GiB");
    expect(info.bytes).toBe(1073741824);
  });
});

// ── Regex ──────────────────────────────────────────────────
describe("isRegex", () => {
  it("detects simple regex", () => {
    expect(T.isRegex("/hello/")).toBe(true);
  });
  it("detects regex with flags", () => {
    expect(T.isRegex("/\\d+/gi")).toBe(true);
  });
  it("detects complex regex", () => {
    expect(T.isRegex("/^[a-z]+$/i")).toBe(true);
  });
  it("rejects non-regex", () => {
    expect(T.isRegex("hello")).toBe(false);
  });
  it("rejects too short", () => {
    expect(T.isRegex("//")).toBe(false);
  });
  it("rejects invalid regex", () => {
    expect(T.isRegex("/[/")).toBe(false);
  });
});

describe("regexInfo", () => {
  it("extracts pattern and flags", () => {
    const info = T.regexInfo("/\\d+/gi");
    expect(info.pattern).toBe("\\d+");
    expect(info.flags).toBe("gi");
    expect(info.flagDescs).toContain("global");
    expect(info.flagDescs).toContain("case-insensitive");
  });
});

// ── Coordinate ─────────────────────────────────────────────
describe("isCoordinate", () => {
  it("detects lat,lng with comma", () => {
    expect(T.isCoordinate("40.7128, -74.0060")).toBe(true);
  });
  it("detects lat lng with space", () => {
    expect(T.isCoordinate("35.6762 139.6503")).toBe(true);
  });
  it("rejects out-of-range lat", () => {
    expect(T.isCoordinate("91.0, 0")).toBe(false);
  });
  it("rejects out-of-range lng", () => {
    expect(T.isCoordinate("0, 181.0")).toBe(false);
  });
  it("rejects plain text", () => {
    expect(T.isCoordinate("not coords")).toBe(false);
  });
});

describe("coordInfo", () => {
  it("parses coordinates", () => {
    const info = T.coordInfo("40.7128, -74.0060");
    expect(info.lat).toBeCloseTo(40.7128);
    expect(info.lng).toBeCloseTo(-74.006);
    expect(info.dms).toContain("N");
    expect(info.dms).toContain("W");
  });
});

// ── MIME Type ──────────────────────────────────────────────
describe("isMimeType", () => {
  it("detects application/json", () => {
    expect(T.isMimeType("application/json")).toBe(true);
  });
  it("detects text/html", () => {
    expect(T.isMimeType("text/html")).toBe(true);
  });
  it("detects image/svg+xml", () => {
    expect(T.isMimeType("image/svg+xml")).toBe(true);
  });
  it("rejects plain text", () => {
    expect(T.isMimeType("hello")).toBe(false);
  });
  it("rejects invalid type", () => {
    expect(T.isMimeType("invalid/")).toBe(false);
  });
});

describe("mimeInfo", () => {
  it("describes application/json", () => {
    const info = T.mimeInfo("application/json");
    expect(info.type).toBe("application");
    expect(info.subtype).toBe("json");
    expect(info.description).toBe("JSON data");
  });
  it("describes unknown type", () => {
    const info = T.mimeInfo("application/x-custom");
    expect(info.description).toBe("application content");
  });
});

// ── Math Expression ────────────────────────────────────────
describe("isMathExpr", () => {
  it("detects addition", () => {
    expect(T.isMathExpr("2 + 3")).toBe(true);
  });
  it("detects complex expression", () => {
    expect(T.isMathExpr("(2 + 3) * 4 / 2")).toBe(true);
  });
  it("detects power", () => {
    expect(T.isMathExpr("2 ^ 10")).toBe(true);
  });
  it("rejects plain text", () => {
    expect(T.isMathExpr("hello")).toBe(false);
  });
  it("rejects single number", () => {
    expect(T.isMathExpr("42")).toBe(false);
  });
  it("rejects division by zero (Infinity)", () => {
    expect(T.isMathExpr("1/0")).toBe(false);
  });
  it("rejects malformed and executable input", () => {
    expect(T.isMathExpr("2..3 + 1")).toBe(false);
    expect(T.isMathExpr("globalThis.alert(1)")).toBe(false);
  });
});

describe("mathEval", () => {
  it("evaluates expression", () => {
    const info = T.mathEval("2 + 3 * 4");
    expect(info.result).toBe(14);
  });
  it("evaluates power", () => {
    const info = T.mathEval("2 ^ 10");
    expect(info.result).toBe(1024);
  });
  it("uses right-associative powers and unary operators", () => {
    expect(T.mathEval("2 ^ 3 ^ 2").result).toBe(512);
    expect(T.mathEval("-(2 + 3) * 4").result).toBe(-20);
  });
});

// ── HTTP Status Code ───────────────────────────────────────
describe("isHttpStatus", () => {
  it("detects 200", () => {
    expect(T.isHttpStatus("200")).toBe(true);
  });
  it("detects 404", () => {
    expect(T.isHttpStatus("404")).toBe(true);
  });
  it("detects 500", () => {
    expect(T.isHttpStatus("500")).toBe(true);
  });
  it("rejects unknown code", () => {
    expect(T.isHttpStatus("999")).toBe(false);
  });
  it("rejects non-3-digit", () => {
    expect(T.isHttpStatus("20")).toBe(false);
  });
});

describe("httpStatusInfo", () => {
  it("describes 200", () => {
    const info = T.httpStatusInfo("200");
    expect(info.code).toBe(200);
    expect(info.message).toBe("OK");
    expect(info.category).toBe("Success");
  });
  it("describes 404", () => {
    const info = T.httpStatusInfo("404");
    expect(info.message).toBe("Not Found");
    expect(info.category).toBe("Client Error");
  });
  it("describes 503", () => {
    const info = T.httpStatusInfo("503");
    expect(info.message).toBe("Service Unavailable");
    expect(info.category).toBe("Server Error");
  });
});
