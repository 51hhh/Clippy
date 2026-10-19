import { createHash } from "node:crypto";
import { lstatSync, readFileSync } from "node:fs";
import { dirname, join, resolve, win32 } from "node:path";
import { fileURLToPath } from "node:url";

export const CRT_FILES = new Set([
  "concrt140.dll", "msvcp140.dll", "msvcp140_1.dll", "msvcp140_2.dll",
  "msvcp140_atomic_wait.dll", "msvcp140_codecvt_ids.dll", "vccorlib140.dll",
  "vcruntime140.dll", "vcruntime140_1.dll", "vcruntime140_threads.dll",
]);
const CORE_CRT = ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"];
// 这里只分类系统依赖，不证明 API 在某个 Windows build 上可用。
const OS_DLLS = new Set([
  "advapi32.dll", "avrt.dll", "bcrypt.dll", "bcryptprimitives.dll", "combase.dll",
  "comctl32.dll", "crypt32.dll", "d3d11.dll", "dnsapi.dll", "dwmapi.dll", "gdi32.dll",
  "imm32.dll", "iphlpapi.dll", "kernel32.dll", "msvcrt.dll", "normaliz.dll", "ntdll.dll",
  "ole32.dll", "oleaut32.dll", "powrprof.dll", "propsys.dll", "rpcrt4.dll", "secur32.dll",
  "shell32.dll", "shlwapi.dll", "ucrtbase.dll", "user32.dll", "version.dll", "winmm.dll",
  "wintrust.dll", "ws2_32.dll", "wtsapi32.dll",
]);
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
function requireValue(condition, message) { if (!condition) throw new Error(message); }
function dllName(value) {
  requireValue(typeof value === "string" && /^[a-z0-9_][a-z0-9_.-]*\.dll$/i.test(value), "Invalid DLL name or path alias");
  return value.toLowerCase();
}
function isOsDll(name) {
  return OS_DLLS.has(name) || /^(api-ms-win-|ext-ms-win-)[a-z0-9.-]+\.dll$/.test(name);
}

export function inspectPe(bytes) {
  requireValue(Buffer.isBuffer(bytes), "PE input must be a Buffer");
  const bounds = (at, size) => requireValue(Number.isSafeInteger(at) && at >= 0 && at + size <= bytes.length, "Truncated PE or invalid RVA");
  const u16 = (at) => { bounds(at, 2); return bytes.readUInt16LE(at); };
  const u32 = (at) => { bounds(at, 4); return bytes.readUInt32LE(at); };
  bounds(0, 64);
  requireValue(bytes.toString("ascii", 0, 2) === "MZ", "Invalid DOS header");
  const pe = u32(0x3c); bounds(pe, 24);
  requireValue(bytes.toString("binary", pe, pe + 4) === "PE\0\0", "Invalid PE header");
  requireValue(u16(pe + 4) === 0x8664, "Expected AMD64 PE");
  const sectionsCount = u16(pe + 6), optionalSize = u16(pe + 20), opt = pe + 24;
  bounds(opt, optionalSize);
  requireValue(optionalSize >= 112 && u16(opt) === 0x20b, "Expected PE32+ optional header");
  const directories = u32(opt + 108);
  requireValue(directories <= 16 && optionalSize >= 112 + directories * 8, "Invalid PE data directories");
  const imageBase = bytes.readBigUInt64LE(opt + 24), headers = u32(opt + 60);
  const sections = [];
  requireValue(sectionsCount > 0 && sectionsCount <= 96, "Invalid PE section count");
  for (let i = 0; i < sectionsCount; i++) {
    const at = opt + optionalSize + i * 40; bounds(at, 40);
    const section = { rva: u32(at + 12), size: u32(at + 16), offset: u32(at + 20) };
    bounds(section.offset, section.size); sections.push(section);
  }
  const offset = (rva, size = 1) => {
    if (rva < headers) { requireValue(rva + size <= headers, "RVA exceeds PE headers"); bounds(rva, size); return rva; }
    const candidates = sections.filter((s) => rva >= s.rva && rva + size <= s.rva + s.size);
    requireValue(candidates.length === 1, "Unmapped or ambiguous RVA");
    const at = candidates[0].offset + rva - candidates[0].rva; bounds(at, size); return at;
  };
  const stringAt = (rva) => {
    const at = offset(rva), end = bytes.indexOf(0, at);
    requireValue(end >= at && end - at < 4096, "Unterminated DLL name");
    offset(rva, end - at + 1);
    requireValue(bytes.subarray(at, end).every((v) => v > 0 && v < 128), "Non-ASCII DLL name");
    return dllName(bytes.toString("ascii", at, end));
  };
  const imports = [], delayImports = [];
  const table = (directory, stride, visit) => {
    if (directories <= directory) return;
    const rva = u32(opt + 112 + directory * 8), size = u32(opt + 116 + directory * 8);
    if (rva === 0 && size === 0) return;
    requireValue(rva !== 0 && size >= stride, "Invalid import directory");
    const at = offset(rva, size);
    for (let i = 0; (i + 1) * stride <= size; i++) {
      const values = Array.from({ length: stride / 4 }, (_, j) => u32(at + i * stride + j * 4));
      if (values.every((v) => v === 0)) return;
      visit(values);
    }
    throw new Error("Unterminated import table");
  };
  table(1, 20, (values) => imports.push(stringAt(values[3])));
  table(13, 32, (values) => {
    requireValue((values[0] & ~1) === 0, "Unknown delay import attributes");
    const name = values[0] & 1 ? BigInt(values[1]) : BigInt(values[1]) - imageBase;
    requireValue(name > 0n && name <= 0xffffffffn, "Invalid delay import address");
    delayImports.push(stringAt(Number(name)));
  });
  return { machine: "AMD64", imports, delayImports };
}

export function verifyLocalRuntimeClosure(executable, libraries) {
  const needed = new Set(), visiting = new Set();
  function visit(pe, owner) {
    for (const imported of [...pe.imports, ...pe.delayImports]) {
      const name = dllName(imported);
      if (isOsDll(name)) continue;
      requireValue(libraries.has(name), `Missing app-local dependency ${name} imported by ${owner}`);
      needed.add(name);
      if (!visiting.has(name)) { visiting.add(name); visit(libraries.get(name), name); }
    }
  }
  if (executable) visit(executable, "executable");
  // SDK 的 dot libraries 同样检查，避免只核对当前 EXE 的一个入口。
  for (const [name, pe] of libraries) visit(pe, name);
  return [...needed].sort();
}

function normalizedPath(value) {
  requireValue(typeof value === "string" && value.length > 0, "Missing staged path");
  return (/^[a-z]:[\\/]/i.test(value) ? win32.resolve(value) : resolve(value)).toLowerCase();
}
function version(value) {
  requireValue(typeof value === "string" && /^\d+\.\d+(?:\.\d+){1,2}$/.test(value), "Invalid toolset or file version");
  const parts = value.split(".").map(Number);
  requireValue(parts.every(Number.isSafeInteger), "Invalid toolset or file version");
  return parts;
}
function atLeastVersion(actual, minimum) {
  for (let i = 0; i < 4; i++) {
    const current = actual[i] ?? 0, required = minimum[i] ?? 0;
    if (current !== required) return current > required;
  }
  return true;
}

export function verifyRuntime({ manifestBytes, configuration, expectedManifestSha256, expectedSource, readFile, executableBytes = null, payloadRoot = null }) {
  requireValue(/^[a-f0-9]{64}$/.test(expectedManifestSha256) && sha256(manifestBytes) === expectedManifestSha256, "Runtime provenance hash mismatch");
  const manifest = JSON.parse(manifestBytes.toString("utf8"));
  requireValue(manifest.schema === 1 && manifest.requirement === "WIN-QA-CRT-01", "Unknown runtime provenance schema");
  requireValue(/^[a-f0-9]{40}$/.test(expectedSource) && manifest.sourceSha === expectedSource, "Runtime provenance source mismatch");
  requireValue(Array.isArray(manifest.sourceGitStatus) && manifest.sourceGitStatus.length === 0, "Runtime provenance requires clean source");
  requireValue(Array.isArray(manifest.files) && manifest.files.length > 0, "Runtime file list is empty");
  const toolset = version(manifest.toolsetVersion), libraries = new Map(), seen = new Set();
  requireValue(toolset[0] === 14, "Expected MSVC v14 toolset");
  const resources = configuration?.bundle?.resources;
  requireValue(resources && !Array.isArray(resources) && typeof resources === "object", "Runtime resources must be an object");
  requireValue(normalizedPath(manifest.manifestPath) === normalizedPath(join(manifest.stagingDirectory, "windows-qa-vc-runtime.json"))
    && resources[manifest.manifestPath] === "licenses/windows-qa-vc-runtime.json", "Runtime provenance resource is incomplete");
  const mappings = new Map();
  for (const [sourcePath, destination] of Object.entries(resources)) {
    const target = String(destination).toLowerCase();
    if (target.endsWith(".dll")) {
      const name = dllName(target);
      requireValue(CRT_FILES.has(name) && !mappings.has(name), "Unknown or duplicate CRT resource");
      mappings.set(name, normalizedPath(sourcePath));
    }
  }
  for (const file of manifest.files) {
    const name = dllName(file.name);
    requireValue(CRT_FILES.has(name) && !seen.has(name), "Unknown or duplicate CRT file"); seen.add(name);
    requireValue(normalizedPath(file.stagedPath) === normalizedPath(join(manifest.stagingDirectory, name)), "CRT file escaped staging directory");
    requireValue(mappings.get(name) === normalizedPath(file.stagedPath), `CRT resource must map ${name} to the EXE directory`);
    requireValue(file.signature?.status === "Valid" && /(?:^|,\s*)O=Microsoft Corporation(?:,|$)/.test(file.signature.subject)
      && /^[a-f0-9]{40}$/i.test(file.signature.thumbprint), "CRT publisher/signature rejected");
    const actualVersion = version(file.version);
    requireValue(actualVersion[0] === toolset[0] && atLeastVersion(actualVersion, toolset) && file.isDebug === false, "CRT version/debug build rejected");
    const bytes = readFile(file.stagedPath);
    requireValue(bytes.length === file.bytes && /^[a-f0-9]{64}$/.test(file.sha256) && sha256(bytes) === file.sha256, `CRT file hash mismatch: ${name}`);
    libraries.set(name, inspectPe(bytes));
    if (payloadRoot) {
      const payload = readFile(join(payloadRoot, name));
      requireValue(sha256(payload) === file.sha256, `Deployed CRT file mismatch: ${name}`);
    }
  }
  requireValue(mappings.size === seen.size && CORE_CRT.every((name) => seen.has(name)), "Core CRT files/resources are incomplete");
  if (payloadRoot) requireValue(sha256(readFile(join(payloadRoot, "licenses/windows-qa-vc-runtime.json"))) === expectedManifestSha256, "Deployed provenance mismatch");
  const executable = executableBytes ? inspectPe(executableBytes) : null;
  const required = verifyLocalRuntimeClosure(executable, libraries);
  return { requirement: "WIN-QA-CRT-01", sourceSha: expectedSource, runtimeFiles: seen.size, required,
    executableSha256: executableBytes ? sha256(executableBytes) : null, payloadVerified: Boolean(payloadRoot),
    manifestSha256: expectedManifestSha256, scope: "File/PE deployment contract only; no DLL load or native runtime compatibility claim" };
}

function main() {
  const options = new Map();
  for (let i = 2; i < process.argv.length; i += 2) {
    const key = process.argv[i], value = process.argv[i + 1];
    requireValue(["--manifest", "--config", "--manifest-sha256", "--expected-source", "--executable", "--payload-root"].includes(key)
      && value && !options.has(key), "Invalid or repeated runtime verification argument");
    options.set(key, value);
  }
  for (const key of ["--manifest", "--config", "--manifest-sha256", "--expected-source"]) requireValue(options.has(key), `Missing ${key}`);
  const manifestPath = resolve(options.get("--manifest")), manifestBytes = readFileSync(manifestPath);
  const manifest = JSON.parse(manifestBytes.toString("utf8"));
  requireValue(normalizedPath(manifest.stagingDirectory) === normalizedPath(dirname(manifestPath)), "Provenance must reside in staging directory");
  const plainFile = (path) => {
    const info = lstatSync(path);
    requireValue(info.isFile() && !info.isSymbolicLink(), "Runtime input must be a regular file");
    return readFileSync(path);
  };
  const executable = options.get("--executable"), payload = options.get("--payload-root");
  requireValue(!payload || executable && normalizedPath(payload) === normalizedPath(dirname(resolve(executable))), "Payload must be the EXE directory");
  const result = verifyRuntime({ manifestBytes, configuration: JSON.parse(readFileSync(options.get("--config"), "utf8")),
    expectedManifestSha256: options.get("--manifest-sha256"), expectedSource: options.get("--expected-source"),
    readFile: plainFile, executableBytes: executable ? plainFile(executable) : null, payloadRoot: payload ? resolve(payload) : null });
  console.log(JSON.stringify(result));
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
