"""显式命令行安装公开模型；用户已有 EdgeGNN 只引用，不下载或复制。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import urllib.request

PUBLIC = {
    "det": ("https://huggingface.co/PaddlePaddle/PP-OCRv6_small_det_onnx/resolve/5994b4f1a827310b6d38f6b13716af6381847be0/inference.onnx", "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e", "det.onnx"),
    "rec": ("https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx/resolve/3d2d345e6a299891174f1397a72cdd81331359c7/inference.onnx", "5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634", "rec.onnx"),
    "dictionary": ("https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/2661c7c0ef5c613e8f93c6e93b2e052399f0f854/ppocr/utils/dict/ppocrv6_dict.txt", "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d", "dictionary.txt"),
}
ENGLISH_REC = ("https://huggingface.co/PaddlePaddle/en_PP-OCRv5_mobile_rec_onnx/resolve/3fafbc3b5dcf93dd72add9f48368be8a3a2cd33b/inference.onnx", "b5f833dfc5d0eb71da397b4efa06ebeee9b431b690a47d6af40d77d8eabc557f", "english-rec.onnx")
LINE_ORIENTATION = ("https://huggingface.co/PaddlePaddle/PP-LCNet_x0_25_textline_ori_onnx/resolve/aea1f18d97338aaf84b463f90192f2820524a00a/inference.onnx", "94a6a0a0425f2b5f08b5df72086f2d72abe40f1d22f6d12d2cd83674f11f2ff3", "line-orientation.onnx")


def digest(path):
    hasher = hashlib.sha256(); count = 0
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            count += len(chunk)
            if count > 64 * 1024 * 1024:
                raise ValueError("模型超过64MiB")
            hasher.update(chunk)
    if count == 0:
        raise ValueError("模型为空")
    return hasher.hexdigest()


def download(path, url, expected):
    if path.exists():
        if digest(path) != expected:
            raise ValueError("已有文件SHA不符，请先自行检查；不会覆盖: " + str(path))
        return
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as handle:
            temporary = Path(handle.name)
            with urllib.request.urlopen(url, timeout=30) as response:
                count = 0
                while chunk := response.read(65536):
                    count += len(chunk)
                    if count > 64 * 1024 * 1024:
                        raise ValueError("下载超过模型预算")
                    handle.write(chunk)
        if digest(temporary) != expected:
            raise ValueError("官方模型下载SHA不符")
        # 不覆盖同时出现的用户文件；hardlink 原子地公开已核对的完整文件。
        os.link(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", required=True, type=Path)
    parser.add_argument("--python", required=True, type=Path)
    parser.add_argument("--edge", required=True, type=Path)
    parser.add_argument("--edge-sha256", required=True)
    parser.add_argument("--english-spacing", action="store_true")
    parser.add_argument("--line-orientation", action="store_true")
    args = parser.parse_args()
    runtime, python, edge = args.runtime.absolute(), args.python.absolute(), args.edge.absolute()
    if not python.is_file() or not edge.is_file() or digest(edge) != args.edge_sha256:
        raise ValueError("需要有效的隔离Python和预先核对SHA的本地Edge模型")
    runtime.mkdir(parents=True, exist_ok=True, mode=0o700)
    destination = runtime / "manifest.json"
    if destination.exists():
        raise ValueError("manifest已存在；不会覆盖本地配置: " + str(destination))
    models = {}
    for name, (url, expected, filename) in PUBLIC.items():
        path = runtime / filename
        download(path, url, expected)
        models[name] = {"path": str(path), "sha256": expected}
    if args.english_spacing:
        url, expected, filename = ENGLISH_REC
        path = runtime / filename
        download(path, url, expected)
        models["englishRec"] = {"path": str(path), "sha256": expected}
        english_dictionary = Path(__file__).resolve().with_name("en-ppocrv5-dictionary.txt")
        english_dictionary_sha = "e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6"
        if digest(english_dictionary) != english_dictionary_sha:
            raise ValueError("内置英语字典SHA不符")
        models["englishDictionary"] = {"path": str(english_dictionary), "sha256": english_dictionary_sha}
    if args.line_orientation:
        url, expected, filename = LINE_ORIENTATION
        path = runtime / filename
        download(path, url, expected)
        models["lineOrientation"] = {"path": str(path), "sha256": expected}
    models["edge"] = {"path": str(edge), "sha256": args.edge_sha256}
    if args.english_spacing and args.line_orientation:
        pipeline_id = "ppocrv6-edgegnn-en-ori-v3"
    elif args.english_spacing:
        pipeline_id = "ppocrv6-edgegnn-en-v2"
    elif args.line_orientation:
        pipeline_id = "ppocrv6-edgegnn-ori-v2"
    else:
        pipeline_id = "ppocrv6-edgegnn-v1"
    manifest = {"version": 1, "python": str(python), "script": str(Path(__file__).resolve().with_name("main.py")),
                "pipelineId": pipeline_id, "featureSchema": "clippy-edge-features-v1", "models": models,
                "options": {"bitmapThreshold": .3, "boxThreshold": .5, "unclipRatio": 1.2, "lineThreshold": .6, "layoutThreshold": .52}}
    with destination.open("x", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, ensure_ascii=False)
    print(destination)


if __name__ == "__main__":
    main()
