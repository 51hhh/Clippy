"""一次请求一进程；只读模型/输入，无网络、GUI 或剪贴板副作用。"""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import time

# -I 禁止外部 PYTHONPATH；唯一导入路径是受信脚本所在目录。
sys.path.insert(0, str(Path(__file__).resolve().parent))
from pipeline import recognize, MAX_PNG_BYTES


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--diagnostics", type=Path)
    parser.add_argument("--manifest-sha256")
    args = parser.parse_args()
    header = sys.stdin.buffer.readline(16385)
    if len(header) > 16384 or not header.endswith(b"\n"):
        raise ValueError("protocol_header")
    request = json.loads(header)
    if set(request) != {"version", "requestId", "pngBytes", "deadlineMs"} or request["version"] != 1:
        raise ValueError("protocol_version")
    count, budget = request["pngBytes"], request["deadlineMs"]
    if type(count) is not int or not 0 < count <= MAX_PNG_BYTES or type(budget) is not int or not 0 < budget <= 60000:
        raise ValueError("protocol_budget")
    if not isinstance(request["requestId"], str) or not 1 <= len(request["requestId"]) <= 128:
        raise ValueError("protocol_identity")
    deadline = time.monotonic() + budget / 1000
    png = sys.stdin.buffer.read(count + 1)
    if len(png) != count:
        raise ValueError("protocol_payload")
    with args.manifest.open("rb") as handle:
        raw = handle.read(65537)
    if len(raw) > 65536:
        raise ValueError("manifest_budget")
    if args.manifest_sha256 and hashlib.sha256(raw).hexdigest() != args.manifest_sha256:
        raise ValueError("manifest_changed")
    manifest = json.loads(raw)
    stages = []
    start = time.monotonic()
    trace = None
    if args.diagnostics:
        trace = lambda stage, data: stages.append(
            {
                "stage": stage,
                "elapsedMs": round((time.monotonic() - start) * 1000, 3),
                **data,
            }
        )
    result = recognize(png, manifest, deadline, trace)
    reply = json.dumps({"version": 1, "requestId": request["requestId"], "result": result}, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode()
    if len(reply) > 4 * 1024 * 1024:
        raise ValueError("protocol_output_budget")
    if args.diagnostics:
        with args.diagnostics.open("x", encoding="utf-8") as handle:
            json.dump({"stages": stages, "result": result}, handle, ensure_ascii=False, allow_nan=False)
    sys.stdout.buffer.write(reply)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # 不把图片文字/路径/原始异常栈暴露到 IPC，诊断仅保留稳定类别。
        print("clippy_ocr_failed:" + type(error).__name__ + ":" + (str(error) if isinstance(error, (ValueError, TimeoutError)) else "runtime_failure"), file=sys.stderr)
        sys.exit(1)
