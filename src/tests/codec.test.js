/**
 * codec.js — 编解码工具测试
 */
import { describe, it, expect } from "vitest";
import { __test__ } from "../js/codec.js";

const { _runOp, _md5, _jwtDecode, _urlParse, _tsToDate, _dateToTs, _numBase, REVERSE_MAP } = __test__;

// ─── Base64 ───
describe("Base64", () => {
  it("encode ASCII", async () => {
    expect(await _runOp("base64-encode", "Hello")).toBe("SGVsbG8=");
  });
  it("decode ASCII", async () => {
    expect(await _runOp("base64-decode", "SGVsbG8=")).toBe("Hello");
  });
  it("encode UTF-8 (中文)", async () => {
    expect(await _runOp("base64-encode", "你好")).toBe("5L2g5aW9");
  });
  it("decode UTF-8 (中文)", async () => {
    expect(await _runOp("base64-decode", "5L2g5aW9")).toBe("你好");
  });
  it("empty string", async () => {
    expect(await _runOp("base64-encode", "")).toBe("");
  });
});

// ─── URL encode/decode ───
describe("URL Encoding", () => {
  it("encode special chars", async () => {
    expect(await _runOp("url-encode", "hello world&foo=bar")).toBe("hello%20world%26foo%3Dbar");
  });
  it("decode", async () => {
    expect(await _runOp("url-decode", "hello%20world%26foo%3Dbar")).toBe("hello world&foo=bar");
  });
  it("encode unicode", async () => {
    const result = await _runOp("url-encode", "你好");
    expect(result).toBe("%E4%BD%A0%E5%A5%BD");
  });
});

// ─── HTML entities ───
describe("HTML Entities", () => {
  it("encode", async () => {
    expect(await _runOp("html-encode", '<script>alert("x")</script>')).toBe(
      "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
    );
  });
  it("encode ampersand", async () => {
    expect(await _runOp("html-encode", "a & b")).toBe("a &amp; b");
  });
  it("encode single quotes", async () => {
    expect(await _runOp("html-encode", "it's")).toBe("it&#39;s");
  });
  it("decodes entities without creating user supplied elements", async () => {
    expect(await _runOp("html-decode", '<img src=x onerror="alert(1)">&amp;&lt;')).toBe(
      '<img src=x onerror="alert(1)">&<',
    );
    expect(document.querySelector("img")).toBeNull();
  });
});

// ─── Unicode escape ───
describe("Unicode Escape", () => {
  it("escape CJK", async () => {
    expect(await _runOp("unicode-escape", "你好")).toBe("\\u4f60\\u597d");
  });
  it("ASCII passthrough", async () => {
    expect(await _runOp("unicode-escape", "abc")).toBe("abc");
  });
  it("unescape", async () => {
    expect(await _runOp("unicode-unescape", "\\u4f60\\u597d")).toBe("你好");
  });
  it("mixed unescape", async () => {
    expect(await _runOp("unicode-unescape", "hello\\u0020world")).toBe("hello world");
  });
});

// ─── Hex ───
describe("Hex", () => {
  it("encode", async () => {
    expect(await _runOp("hex-encode", "AB")).toBe("41 42");
  });
  it("decode", async () => {
    expect(await _runOp("hex-decode", "41 42")).toBe("AB");
  });
  it("decode comma-separated", async () => {
    expect(await _runOp("hex-decode", "48,65,6c,6c,6f")).toBe("Hello");
  });
});

// ─── ROT13 ───
describe("ROT13", () => {
  it("basic", async () => {
    expect(await _runOp("rot13", "Hello")).toBe("Uryyb");
  });
  it("round-trip", async () => {
    const encoded = await _runOp("rot13", "Test 123!");
    expect(await _runOp("rot13", encoded)).toBe("Test 123!");
  });
  it("preserves non-alpha", async () => {
    expect(await _runOp("rot13", "123 !@#")).toBe("123 !@#");
  });
});

// ─── MD5 ───
describe("MD5", () => {
  it("empty string", () => {
    expect(_md5("")).toBe("d41d8cd98f00b204e9800998ecf8427e");
  });
  it("hello", () => {
    expect(_md5("hello")).toBe("5d41402abc4b2a76b9719d911017c592");
  });
  it("Hello World", () => {
    expect(_md5("Hello World")).toBe("b10a8db164e0754105b7a99be72e3fe5");
  });
  it("long string (> 64 bytes)", () => {
    const long = "a".repeat(100);
    expect(_md5(long)).toBe("36a92cc94a9e0fa21f625f8bfb007adf");
  });
});

// ─── SHA hashes ───
describe("SHA hashes", () => {
  it("SHA-256 of empty string", async () => {
    const result = await _runOp("sha256", "");
    expect(result).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
  });
  it("SHA-1 of hello", async () => {
    const result = await _runOp("sha1", "hello");
    expect(result).toBe("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
  });
  it("SHA-512 returns 128 hex chars", async () => {
    const result = await _runOp("sha512", "test");
    expect(result).toHaveLength(128);
  });
});

// ─── JSON format/minify ───
describe("JSON", () => {
  it("format", async () => {
    expect(await _runOp("json-format", '{"a":1}')).toBe('{\n  "a": 1\n}');
  });
  it("minify", async () => {
    expect(await _runOp("json-minify", '{\n  "a": 1\n}')).toBe('{"a":1}');
  });
  it("invalid JSON throws", async () => {
    await expect(_runOp("json-format", "{bad}")).rejects.toThrow();
  });
});

// ─── JWT Decode ───
describe("JWT Decode", () => {
  it("valid JWT", () => {
    // Header: {"alg":"HS256","typ":"JWT"}, Payload: {"sub":"1234567890","name":"Test","iat":1516239022}
    const jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IlRlc3QiLCJpYXQiOjE1MTYyMzkwMjJ9.fake-sig";
    // Header 和 Payload 是两个独立字段，各自可单独复制
    const { fields } = _jwtDecode(jwt);
    expect(fields.map((f) => f.label)).toEqual(["Header", "Payload"]);
    expect(fields[0].value).toContain("HS256");
    expect(fields[1].value).toContain("1234567890");
    expect(fields[1].value).toContain("Test");
  });
  it("invalid JWT throws", () => {
    expect(() => _jwtDecode("not-a-jwt")).toThrow("Invalid JWT");
  });
});

// ─── URL Parse ───
describe("URL Parse", () => {
  it("basic URL", () => {
    const { fields } = _urlParse("https://example.com/path?q=test&lang=en#section");
    expect(fields).toEqual([
      { label: "Protocol", value: "https:", group: undefined },
      { label: "Host", value: "example.com", group: undefined },
      { label: "Path", value: "/path", group: undefined },
      // 查询参数本身就是键值对，键名不翻译，只用分组标题隔开
      { label: "q", value: "test", group: "Query Parameters" },
      { label: "lang", value: "en", group: "Query Parameters" },
      { label: "Fragment", value: "#section", group: undefined },
    ]);
  });
  it("URL with port", () => {
    const { fields } = _urlParse("http://localhost:3000/api");
    expect(fields).toContainEqual({ label: "Port", value: "3000", group: undefined });
  });
  it("omits fields the URL does not have", () => {
    const { fields } = _urlParse("https://example.com/");
    expect(fields.map((f) => f.label)).toEqual(["Protocol", "Host", "Path"]);
  });
  it("invalid URL throws", () => {
    expect(() => _urlParse("not a url")).toThrow();
  });
});

// ─── Timestamp ←→ Date ───
describe("Timestamp", () => {
  it("seconds → date", () => {
    // 三种写法各成一对键值：只想要 ISO 时就不该连带复制另外两行
    const { fields } = _tsToDate("0");
    expect(fields.map((f) => f.label)).toEqual(["Local", "UTC", "ISO 8601"]);
    expect(fields.find((f) => f.label === "ISO 8601").value).toBe("1970-01-01T00:00:00.000Z");
    expect(fields.find((f) => f.label === "UTC").value).toContain("1970");
  });
  it("milliseconds → date", () => {
    const { fields } = _tsToDate("1700000000000");
    expect(fields.find((f) => f.label === "ISO 8601").value).toContain("2023");
  });
  it("invalid ts throws", () => {
    expect(() => _tsToDate("abc")).toThrow("Invalid timestamp");
  });
  it("date → timestamp", () => {
    const { fields } = _dateToTs("2023-01-01T00:00:00Z");
    expect(fields).toEqual([
      { label: "Seconds", value: "1672531200", group: undefined },
      { label: "Milliseconds", value: "1672531200000", group: undefined },
    ]);
  });
  it("invalid date throws", () => {
    expect(() => _dateToTs("not-a-date")).toThrow("Invalid date");
  });
});

// ─── Number Base ───
describe("Number Base", () => {
  const valueOf = (result, label) => result.fields.find((f) => f.label === label).value;

  it("decimal to all bases", () => {
    const result = _numBase("255");
    expect(result.fields.map((f) => f.label)).toEqual(["Decimal", "Hex", "Binary", "Octal"]);
    expect(valueOf(result, "Decimal")).toBe("255");
    expect(valueOf(result, "Hex")).toBe("0xFF");
    expect(valueOf(result, "Binary")).toBe("0b11111111");
    expect(valueOf(result, "Octal")).toBe("0o377");
  });
  it("hex input", () => {
    expect(valueOf(_numBase("0xFF"), "Decimal")).toBe("255");
  });
  it("binary input", () => {
    expect(valueOf(_numBase("0b1010"), "Decimal")).toBe("10");
  });
  it("octal input", () => {
    expect(valueOf(_numBase("0o17"), "Decimal")).toBe("15");
  });
  it("invalid number throws", () => {
    expect(() => _numBase("xyz")).toThrow("Invalid number");
  });
});

// ─── REVERSE_MAP ───
describe("REVERSE_MAP", () => {
  it("all pairs are bidirectional", () => {
    for (const [k, v] of Object.entries(REVERSE_MAP)) {
      expect(REVERSE_MAP[v]).toBe(k);
    }
  });
});

// ─── Unknown operation ───
describe("Unknown operation", () => {
  it("returns error message", async () => {
    const result = await _runOp("nonexistent", "test");
    expect(result).toContain("Unknown operation");
  });
});
