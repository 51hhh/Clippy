#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { mkdirSync, statSync, writeFileSync } from "node:fs";
import { arch, cpus, platform, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDir, "..");

function usage() {
  console.log(`Usage: node scripts/benchmark-recording-encoders.mjs [options]

Options:
  --duration <seconds>  Fixture duration (default: 6)
  --width <pixels>      Width (default: 1280)
  --height <pixels>     Height (default: 720)
  --fps <number>        Frame rate (default: 30)
  --output <directory>  Artifact directory (default: a new /tmp directory)
  --help                Show this help

This is an opt-in engineering benchmark. It requires ffmpeg/ffprobe and does not
run in ci-local.sh or prove that the corresponding codec can be embedded.`);
}

function parsePositiveNumber(raw, name, integer = false) {
  const value = Number(raw);
  if (!Number.isFinite(value) || value <= 0 || (integer && !Number.isInteger(value))) {
    throw new Error(`${name} must be a positive ${integer ? "integer" : "number"}`);
  }
  return value;
}

function parseArgs(argv) {
  const options = { duration: 6, width: 1280, height: 720, fps: 30, output: null };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help") {
      usage();
      process.exit(0);
    }
    const next = argv[index + 1];
    if (!next) throw new Error(`${argument} requires a value`);
    switch (argument) {
      case "--duration":
        options.duration = parsePositiveNumber(next, argument);
        break;
      case "--width":
        options.width = parsePositiveNumber(next, argument, true);
        break;
      case "--height":
        options.height = parsePositiveNumber(next, argument, true);
        break;
      case "--fps":
        options.fps = parsePositiveNumber(next, argument, true);
        break;
      case "--output":
        options.output = resolve(next);
        break;
      default:
        throw new Error(`unknown option: ${argument}`);
    }
    index += 1;
  }
  if (options.width > 7680 || options.height > 4320 || options.fps > 120 || options.duration > 600) {
    throw new Error("benchmark dimensions, fps, or duration exceed the safety budget");
  }
  options.output ??= join(tmpdir(), `clippy-recording-codecs-${Date.now()}`);
  return options;
}

function run(command, args, { allowFailure = false } = {}) {
  const started = performance.now();
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const elapsedMs = performance.now() - started;
  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) {
    throw new Error(`${command} exited ${result.status}\n${result.stderr}`);
  }
  return { ...result, elapsedMs };
}

function requireTool(name) {
  const result = run(name, ["-version"], { allowFailure: true });
  if (result.status !== 0) throw new Error(`${name} is required`);
}

function escapeFilterPath(value) {
  return value.replaceAll("\\", "\\\\").replaceAll(":", "\\:").replaceAll("'", "\\'");
}

function fixtureFilter(options) {
  const font = escapeFilterPath(
    join(projectRoot, "src-tauri", "assets", "fonts", "NotoSansCJKsc-Medium.otf"),
  );
  const { width, height, fps, duration } = options;
  const small = Math.max(14, Math.round(height / 45));
  const large = Math.max(24, Math.round(height / 24));
  return [
    `testsrc2=size=${width}x${height}:rate=${fps}:duration=${duration}`,
    "format=yuv420p",
    "drawbox=x=24:y=24:w=iw-48:h=ih-48:color=white@0.92:t=fill",
    "drawbox=x=48:y=58:w=iw-96:h=72:color=0xE9EDF5:t=fill",
    `drawtext=fontfile='${font}':text='Clippy UI 0123456789 ABC xyz':fontcolor=0x111827:fontsize=${large}:x=64:y=76`,
    `drawtext=fontfile='${font}':text='中文 English 日本語  ¥ €  + - × ÷  (a+b)^2':fontcolor=0x1F2937:fontsize=${small}:x=64:y=h-mod(t*150\\,h+160)`,
    `drawtext=fontfile='${font}':text='Small text 11 12 13 14 15 16 17 18 19':fontcolor=0x374151:fontsize=${small}:x=w-t*180:y=h-92`,
    "drawgrid=width=96:height=64:thickness=1:color=0x64748B@0.28",
  ].join(",");
}

function parseBenchmark(stderr) {
  const match = stderr.match(/bench: utime=([0-9.]+)s stime=([0-9.]+)s rtime=([0-9.]+)s[\s\S]*?maxrss=(\d+)KiB/);
  return match
    ? { userSeconds: Number(match[1]), systemSeconds: Number(match[2]), realSeconds: Number(match[3]), maxRssKiB: Number(match[4]) }
    : null;
}

function metric(candidate, reference, kind) {
  const result = run("ffmpeg", [
    "-nostdin", "-hide_banner", "-loglevel", "info",
    "-i", candidate, "-i", reference,
    "-lavfi", kind, "-f", "null", "-",
  ]);
  if (kind === "ssim") {
    const value = result.stderr.match(/All:([0-9.]+)/)?.[1];
    return value ? Number(value) : null;
  }
  const value = result.stderr.match(/average:([0-9.]+)/)?.[1];
  return value ? Number(value) : null;
}

function probeVideo(path) {
  const result = run("ffprobe", [
    "-v", "error", "-count_frames", "-select_streams", "v:0",
    "-show_entries", "stream=codec_name,width,height,avg_frame_rate,nb_read_frames:format=duration",
    "-of", "json", path,
  ]);
  const payload = JSON.parse(result.stdout);
  const stream = payload.streams?.[0];
  if (!stream) throw new Error(`ffprobe found no video stream in ${path}`);
  return {
    codec: stream.codec_name,
    width: Number(stream.width),
    height: Number(stream.height),
    averageFrameRate: stream.avg_frame_rate,
    decodedFrames: Number(stream.nb_read_frames),
    durationSeconds: Number(payload.format?.duration),
  };
}

const candidates = [
  {
    id: "mjpeg-avi",
    encoder: "mjpeg",
    extension: "avi",
    args: ["-c:v", "mjpeg", "-q:v", "3", "-pix_fmt", "yuvj420p"],
    eligibility: "diagnostic-only",
  },
  {
    id: "vp9-webm",
    encoder: "libvpx-vp9",
    extension: "webm",
    args: ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "6", "-row-mt", "1", "-crf", "30", "-b:v", "0"],
    eligibility: "embed-build-pending",
  },
  {
    id: "av1-matroska",
    encoder: "librav1e",
    extension: "mkv",
    args: ["-c:v", "librav1e", "-speed", "10", "-qp", "80"],
    eligibility: "realtime-and-package-pending",
  },
];

function markdown(report) {
  const lines = [
    "# Clippy recording codec benchmark",
    "",
    `Fixture: ${report.fixture.width}×${report.fixture.height}, ${report.fixture.fps} fps, ${report.fixture.durationSeconds}s`,
    `Host: ${report.environment.platform}/${report.environment.arch}, ${report.environment.cpu}; ${report.environment.ffmpegVersion}`,
    "",
    "| Candidate | Status | Wall s | Max RSS MiB | Size MiB | SSIM | PSNR dB | Eligibility |",
    "|---|---:|---:|---:|---:|---:|---:|---|",
  ];
  for (const item of report.candidates) {
    lines.push(
      `| ${item.id} | ${item.status} | ${item.wallSeconds ?? "—"} | ${item.maxRssMiB ?? "—"} | ${item.sizeMiB ?? "—"} | ${item.ssim ?? "—"} | ${item.psnrDb ?? "—"} | ${item.eligibility} |`,
    );
  }
  lines.push("", "Generated artifacts are local engineering evidence and are not release approval.", "");
  return lines.join("\n");
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  requireTool("ffmpeg");
  requireTool("ffprobe");
  mkdirSync(options.output, { recursive: true });
  const encoderList = run("ffmpeg", ["-nostdin", "-hide_banner", "-encoders"]).stdout;
  const ffmpegVersion = run("ffmpeg", ["-version"]).stdout.split("\n", 1)[0];
  const reference = join(options.output, "reference-ffv1.mkv");
  const source = fixtureFilter(options);
  run("ffmpeg", [
    "-nostdin", "-hide_banner", "-loglevel", "warning", "-y",
    "-f", "lavfi", "-i", source, "-an", "-c:v", "ffv1", "-level", "3", reference,
  ]);

  const report = {
    generatedAt: new Date().toISOString(),
    outputDirectory: options.output,
    environment: {
      platform: platform(),
      arch: arch(),
      cpu: cpus()[0]?.model ?? "unknown",
      logicalCpus: cpus().length,
      ffmpegVersion,
    },
    fixture: {
      width: options.width,
      height: options.height,
      fps: options.fps,
      durationSeconds: options.duration,
      reference,
      referenceSizeMiB: Number((statSync(reference).size / 1024 / 1024).toFixed(2)),
    },
    candidates: [],
  };
  for (const candidate of candidates) {
    if (!encoderList.includes(candidate.encoder)) {
      report.candidates.push({
        id: candidate.id,
        status: "unavailable",
        eligibility: candidate.eligibility,
      });
      continue;
    }
    const output = join(options.output, `${candidate.id}.${candidate.extension}`);
    const encoded = run("ffmpeg", [
      "-nostdin", "-hide_banner", "-loglevel", "info", "-benchmark", "-y",
      "-i", reference, "-an", ...candidate.args, output,
    ], { allowFailure: true });
    if (encoded.status !== 0) {
      report.candidates.push({
        id: candidate.id,
        status: "failed",
        eligibility: candidate.eligibility,
        error: encoded.stderr.slice(-4000),
      });
      continue;
    }
    const benchmark = parseBenchmark(encoded.stderr);
    report.candidates.push({
      id: candidate.id,
      status: "measured",
      output,
      eligibility: candidate.eligibility,
      wallSeconds: Number((encoded.elapsedMs / 1000).toFixed(3)),
      userSeconds: benchmark?.userSeconds ?? null,
      systemSeconds: benchmark?.systemSeconds ?? null,
      maxRssMiB: benchmark ? Number((benchmark.maxRssKiB / 1024).toFixed(1)) : null,
      sizeMiB: Number((statSync(output).size / 1024 / 1024).toFixed(2)),
      probe: probeVideo(output),
      ssim: metric(output, reference, "ssim"),
      psnrDb: metric(output, reference, "psnr"),
    });
  }
  writeFileSync(join(options.output, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  writeFileSync(join(options.output, "report.md"), markdown(report));
  console.log(markdown(report));
  console.log(`Artifacts: ${options.output}`);
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
