#!/usr/bin/env python3
"""构建机准备离线 OCR 资源，安装后的应用直接运行原生 worker。"""

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BUNDLE = ROOT / "target/packager/slint-ocr"
DEFAULT_CACHE = ROOT / "target/packager/slint-ocr-cache"
MODEL_PINS = ROOT / "crates/clippy-ocr-protocol/models.json"
ORT_VERSION = "1.24.2"
ORT_SOURCE = "058787ceead760166e3c50a0a4cba8a833a6f53f"
MAX_ASSET = 64 * 1024 * 1024
RUST_TARGETS = {
    "linux-x64": "x86_64-unknown-linux-gnu",
    "windows-x64": "x86_64-pc-windows-msvc",
    "macos-aarch64": "aarch64-apple-darwin",
    "macos-x64": "x86_64-apple-darwin",
}
RUST_OCR_LICENSES = {
    "oar-ocr": ("LICENSE",),
    "ort": ("LICENSE-APACHE", "LICENSE-MIT"),
    "ort-sys": ("LICENSE-APACHE", "LICENSE-MIT"),
}
PADDLE_LICENSE = (
    "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/2661c7c0ef5c613e8f93c6e93b2e052399f0f854/LICENSE",
    "3840c5c0c61c294264d2dd77b8777be6ddd90121ef4e0e64abcd22edea581d6e",
)
WHEELS = {
    "linux-x64": (
        "https://files.pythonhosted.org/packages/0d/74/a1913b3a0fc2f27fe1751e9545745a3f35fd7833e3438a4208b4e215778f/onnxruntime-1.24.2-cp314-cp314-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl",
        "2658b3ce6cb33bdeddfcd74c6da509510310717611220cf2106e6c401febabe5",
        {"libonnxruntime.so.1.24.2": "libonnxruntime.so",
         "libonnxruntime_providers_shared.so": "libonnxruntime_providers_shared.so"},
    ),
    "windows-x64": (
        "https://files.pythonhosted.org/packages/0a/bd/fca80d282bca9848b2c8e101c764432dd61a0e9d2377d1c8b3bab13235d0/onnxruntime-1.24.2-cp314-cp314-win_amd64.whl",
        "45b4f68ffec95b2cc0dc96b2b413f69ace9a80a0e5400023c5ac61f73a7a3fdf",
        {"onnxruntime.dll": "onnxruntime.dll",
         "onnxruntime_providers_shared.dll": "onnxruntime_providers_shared.dll"},
    ),
}


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(cache, url, expected):
    asset = cache / expected
    if not asset.exists():
        with tempfile.TemporaryDirectory(dir=cache) as download:
            partial = Path(download) / "asset"
            with urllib.request.urlopen(url, timeout=45) as response, partial.open("wb") as output:
                remaining = MAX_ASSET
                while chunk := response.read(1024 * 1024):
                    remaining -= len(chunk)
                    if remaining < 0:
                        raise ValueError(f"OCR download exceeds the asset limit: {url}")
                    output.write(chunk)
            if sha256(partial) != expected:
                raise ValueError(f"OCR download checksum mismatch: {url}")
            partial.replace(asset)
    if asset.stat().st_size > MAX_ASSET or sha256(asset) != expected:
        raise ValueError(f"Cached OCR asset checksum mismatch: {asset}")
    return asset


def host_platform():
    machine = platform.machine().lower()
    if sys.platform == "darwin":
        return {"arm64": "macos-aarch64", "x86_64": "macos-x64"}[machine]
    if machine in ("x86_64", "amd64"):
        return {"linux": "linux-x64", "win32": "windows-x64"}[sys.platform]
    raise ValueError(f"Unsupported OCR package architecture: {sys.platform}/{machine}")


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def smoke_test(bundle, executable):
    width, height = 192, 64
    image = struct.pack("<4sQII", b"OCR1", 1, width, height) + b"\xff" * (width * height * 4)
    process = run(str(bundle / executable), str(bundle), input=image, capture_output=True, timeout=60,
                  env={**os.environ, "ORT_DISABLE_TELEMETRY": "1"})
    (size,) = struct.unpack_from("<I", process.stdout)
    if size != len(process.stdout) - 4 or json.loads(process.stdout[4:]) != {"id": 1, "result": {"Ok": []}}:
        raise ValueError(f"OCR bundle did not initialize or handle an empty image: {process.stdout[4:]!r}")


def verify_macos_binary(path, arch, maximum):
    if run("lipo", "-archs", str(path), capture_output=True, text=True).stdout.split() != [arch]:
        raise ValueError(f"Wrong macOS OCR architecture: {path}")
    commands = run("otool", "-l", str(path), capture_output=True, text=True).stdout
    minimums = re.findall(r"(?m)^\s*minos\s+(\d+\.\d+(?:\.\d+)?)\s*$", commands)
    if len(minimums) != 1 or tuple(map(int, minimums[0].split(".")[:2])) > maximum:
        raise ValueError(f"{path} exceeds macOS {maximum[0]}.{maximum[1]}: {minimums}")


def build_macos_ort(cache, arch, bundle):
    # 上游二进制的部署目标是 14.0，源码构建保留两个架构的 11.0 合同。
    source = cache / f"onnxruntime-{ORT_SOURCE}"
    source.mkdir(exist_ok=True)
    if not (source / "VERSION_NUMBER").is_file():
        run("git", "init", "--quiet", str(source))
        run("git", "-C", str(source), "fetch", "--depth", "1", "https://github.com/microsoft/onnxruntime.git", ORT_SOURCE)
        run("git", "-C", str(source), "checkout", "--quiet", "--detach", "FETCH_HEAD")
    if run("git", "-C", str(source), "rev-parse", "HEAD", capture_output=True, text=True).stdout.strip() != ORT_SOURCE:
        raise ValueError("Cached ONNX Runtime source revision mismatch")
    run("git", "-C", str(source), "submodule", "update", "--init", "--depth", "1", "--", "cmake/external/onnx")
    build = cache / f"ort-{ORT_SOURCE}-{arch}"
    run("cmake", "-S", str(source / "cmake"), "-B", str(build),
        "-DCMAKE_BUILD_TYPE=Release", f"-DCMAKE_OSX_ARCHITECTURES={arch}",
        "-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0", f"-DPython_EXECUTABLE={sys.executable}",
        "-Donnxruntime_BUILD_SHARED_LIB=ON", "-Donnxruntime_BUILD_UNIT_TESTS=OFF")
    run("cmake", "--build", str(build), "--target", "onnxruntime", "--parallel", "3")
    dylib = build / "libonnxruntime.dylib"
    verify_macos_binary(dylib, arch, (11, 0))
    shutil.copyfile(dylib, bundle / dylib.name)
    for name in ("LICENSE", "ThirdPartyNotices.txt"):
        shutil.copyfile(source / name, bundle / "licenses" / ("ONNXRuntime-" + name))
    return {"commit": ORT_SOURCE, "deploymentTarget": "11.0"}


def stage_runtime(cache, system, bundle, windows_crt):
    if system.startswith("macos-"):
        return build_macos_ort(cache, "arm64" if system == "macos-aarch64" else "x86_64", bundle)
    url, checksum, libraries = WHEELS[system]
    with zipfile.ZipFile(fetch(cache, url, checksum)) as archive:
        entries = {"capi/" + source: target for source, target in libraries.items()}
        for name in ("LICENSE", "ThirdPartyNotices.txt"):
            entries[name] = "licenses/ONNXRuntime-" + name
        for source, target in entries.items():
            member = archive.getinfo("onnxruntime/" + source)
            if not 0 < member.file_size <= MAX_ASSET:
                raise ValueError(f"Invalid ONNX Runtime archive member: {source}")
            with archive.open(member) as original, (bundle / target).open("wb") as output:
                shutil.copyfileobj(original, output)
    if system == "windows-x64":
        manifest = json.loads(windows_crt.read_text(encoding="utf-8-sig"))
        source = run("git", "rev-parse", "HEAD", cwd=ROOT, capture_output=True, text=True).stdout.strip()
        if manifest["sourceSha"] != source:
            raise ValueError("Windows CRT manifest does not match the source SHA")
        for entry in manifest["files"]:
            name = entry["name"]
            staged = Path(entry["stagedPath"])
            if not re.fullmatch(r"[a-z0-9_]+\.dll", name) or sha256(staged) != entry["sha256"]:
                raise ValueError(f"Invalid staged Windows CRT file: {name}")
            shutil.copyfile(staged, bundle / name)
        run("node", str(ROOT / "scripts/verify-slint-ocr-pe.mjs"), str(bundle))
    return {"archiveSha256": checksum}


def assets_in(path):
    return {p.relative_to(path).as_posix(): sha256(p) for p in sorted(path.rglob("*"))
            if p.is_file() and p != path / "OCR-BUNDLE.json"}


def stage_rust_ocr_licenses(bundle, system):
    output = run("cargo", "metadata", "--locked", "--format-version", "1",
                 "--manifest-path", str(ROOT / "apps/ocr-worker/Cargo.toml"),
                 "--filter-platform", RUST_TARGETS[system], capture_output=True).stdout
    for package in json.loads(output)["packages"]:
        name = package["name"]
        for license_file in RUST_OCR_LICENSES.get(name, ()):
            source = Path(package["manifest_path"]).with_name(license_file)
            shutil.copyfile(source, bundle / "licenses" / f"{name}-{license_file}")


def verify_bundle(bundle):
    info = json.loads((bundle / "OCR-BUNDLE.json").read_text(encoding="utf-8"))
    if info["schema"] != 1 or info["runtime"]["version"] != ORT_VERSION or info["platform"] != host_platform():
        raise ValueError("OCR bundle version or platform mismatch")
    staged = assets_in(bundle)
    expected_files = info["files"]
    changed = sorted(name for name in staged.keys() | expected_files.keys()
                     if staged.get(name) != expected_files.get(name))
    if changed:
        raise ValueError(f"OCR bundle files differ from the prepared inventory: {', '.join(changed)}")
    system = info["platform"]
    native_files = WHEELS[system][2].values() if system in WHEELS else ("libonnxruntime.dylib",)
    required = {worker_name(system), *native_files, "licenses/ONNXRuntime-LICENSE",
                "licenses/ONNXRuntime-ThirdPartyNotices.txt", "licenses/PaddleOCR-Apache-2.0.txt",
                "licenses/PaddleOCR-model-sources.json"}
    required.update(f"licenses/{name}-{license_file}" for name, files in RUST_OCR_LICENSES.items()
                    for license_file in files)
    if not required <= staged.keys():
        raise ValueError("OCR bundle is missing a worker, runtime library or license")
    for name, _, expected in json.loads(MODEL_PINS.read_text(encoding="utf-8")):
        if staged.get("models/" + name) != expected:
            raise ValueError(f"OCR bundle model mismatch: {name}")


def worker_name(system):
    return "clippy-ocr-worker.exe" if system == "windows-x64" else "clippy-ocr-worker"


def prepare(args):
    system = host_platform()
    if system == "windows-x64" and args.windows_crt is None:
        raise ValueError("Windows OCR packaging requires the staged CRT manifest")
    cache, bundle = args.cache.absolute(), args.bundle.absolute()
    cache.mkdir(parents=True, exist_ok=True)
    bundle.parent.mkdir(parents=True, exist_ok=True)
    executable = worker_name(system)
    with tempfile.TemporaryDirectory(dir=bundle.parent) as temporary:
        stage = Path(temporary) / "staged"
        (stage / "models").mkdir(parents=True)
        (stage / "licenses").mkdir()
        shutil.copy2(args.worker, stage / executable)
        for name, url, checksum in json.loads(MODEL_PINS.read_text(encoding="utf-8")):
            shutil.copyfile(fetch(cache, url, checksum), stage / "models" / name)
        shutil.copyfile(fetch(cache, *PADDLE_LICENSE), stage / "licenses/PaddleOCR-Apache-2.0.txt")
        shutil.copyfile(MODEL_PINS, stage / "licenses/PaddleOCR-model-sources.json")
        stage_rust_ocr_licenses(stage, system)
        runtime = stage_runtime(cache, system, stage, args.windows_crt)
        if system == "linux-x64":
            # 预设 linuxdeploy 的同目录 RUNPATH，使 deb 与 AppImage 的资源摘要一致。
            for name in (executable, "libonnxruntime_providers_shared.so"):
                run("patchelf", "--set-rpath", "$ORIGIN", str(stage / name))
        elif system.startswith("macos-"):
            arch = "arm64" if system == "macos-aarch64" else "x86_64"
            maximum = tuple(map(int, args.macos_worker_target.split(".")))
            verify_macos_binary(stage / executable, arch, maximum)
        smoke_test(stage, executable)
        info = {"schema": 1, "platform": system, "runtime": {"version": ORT_VERSION, **runtime},
                "files": assets_in(stage)}
        (stage / "OCR-BUNDLE.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
        verify_bundle(stage)
        previous = Path(temporary) / "previous"
        if bundle.exists():
            if bundle.is_symlink() or not (bundle / "OCR-BUNDLE.json").is_file():
                raise ValueError(f"Refusing to replace a directory that is not an OCR bundle: {bundle}")
            bundle.replace(previous)
        try:
            stage.replace(bundle)
        except OSError:
            if previous.exists():
                previous.replace(bundle)
            raise
    print(f"OCR bundle prepared and tested: {bundle}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path)
    parser.add_argument("--bundle", type=Path, default=DEFAULT_BUNDLE)
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--windows-crt", type=Path)
    parser.add_argument("--macos-worker-target", default="11.0", choices=("11.0", "12.3"))
    parser.add_argument("--smoke", action="store_true", help="run the worker in an extracted or signed installer payload")
    args = parser.parse_args()
    if args.worker is not None:
        prepare(args)
    elif args.smoke:
        smoke_test(args.bundle.resolve(), worker_name(host_platform()))
        print(f"OCR worker finished from installer payload: {args.bundle}")
    else:
        verify_bundle(args.bundle)


if __name__ == "__main__":
    main()
