import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { CRT_FILES, inspectPe, verifyLocalRuntimeClosure } from "./verify-windows-qa-runtime.mjs";

const bundle = process.argv[2];
const runtime = new Set([...CRT_FILES, "onnxruntime.dll", "onnxruntime_providers_shared.dll"]);
const libraries = new Map();
for (const entry of readdirSync(bundle)) {
  if (!entry.toLowerCase().endsWith(".dll")) continue;
  if (!runtime.has(entry)) throw new Error(`Unexpected OCR DLL: ${entry}`);
  libraries.set(entry, inspectPe(readFileSync(join(bundle, entry))));
}
for (const name of ["onnxruntime.dll", "onnxruntime_providers_shared.dll"]) {
  if (!libraries.has(name)) throw new Error(`Missing OCR DLL: ${name}`);
}
const worker = inspectPe(readFileSync(join(bundle, "clippy-ocr-worker.exe")));
console.log(verifyLocalRuntimeClosure(worker, libraries).join(", "));
