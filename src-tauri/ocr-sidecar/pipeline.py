"""本地 CPU OCR 真推理：det → EdgeGNN → crop → rec → CTC。"""
import hashlib
import math
import time
from pathlib import Path

import cv2
import numpy as np
import onnxruntime as ort
import pyclipper

from edge_features import build_features, SCHEMA, MAX_NODES
from layout_groups import group_lines
from visual_paragraphs import merge_visual_paragraphs

MAX_PIXELS = 32 * 1024 * 1024
MAX_DIMENSION = 16384
MAX_PNG_BYTES = 64 * 1024 * 1024
MAX_REC_WIDTH = 4096
MAX_REC_PIXELS = 48 * MAX_REC_WIDTH * 128
EXPECTED = {
    "det": "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e",
    "rec": "5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634",
    "dictionary": "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d",
}
DEFAULTS = {"bitmapThreshold": .3, "boxThreshold": .5, "unclipRatio": 1.2, "lineThreshold": .6, "layoutThreshold": .52}


def check_deadline(deadline):
    if time.monotonic() >= deadline:
        raise TimeoutError("ocr_deadline")


def validate_manifest(manifest):
    if manifest.get("version") != 1 or manifest.get("featureSchema") != SCHEMA:
        raise ValueError("manifest_schema")
    if not isinstance(manifest.get("pipelineId"), str) or not 1 <= len(manifest["pipelineId"]) <= 128:
        raise ValueError("pipeline_identity")
    options = dict(DEFAULTS)
    supplied = manifest.get("options", {})
    if not isinstance(supplied, dict) or set(supplied) - set(DEFAULTS):
        raise ValueError("manifest_options")
    options.update(supplied)
    for key, value in options.items():
        maximum = 3 if key == "unclipRatio" else 1
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 < value <= maximum:
            raise ValueError("manifest_options")
    for name in ["det", "rec", "dictionary", "edge"]:
        record = manifest.get("models", {}).get(name, {})
        path = Path(record.get("path", ""))
        if not path.is_absolute() or not path.is_file() or not 0 < path.stat().st_size <= 64 * 1024 * 1024:
            raise ValueError("model_missing_or_size")
        digest = hashlib.sha256()
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        if digest.hexdigest() != record.get("sha256") or (name in EXPECTED and digest.hexdigest() != EXPECTED[name]):
            raise ValueError("model_hash_mismatch")
    return options


def decode_png(png):
    if len(png) > MAX_PNG_BYTES or len(png) < 33 or png[:8] != b"\x89PNG\r\n\x1a\n" or png[12:16] != b"IHDR":
        raise ValueError("invalid_png")
    width, height = int.from_bytes(png[16:20], "big"), int.from_bytes(png[20:24], "big")
    if not width or not height or max(width, height) > MAX_DIMENSION or width * height > MAX_PIXELS:
        raise ValueError("image_budget")
    image = cv2.imdecode(np.frombuffer(png, np.uint8), cv2.IMREAD_UNCHANGED)
    if image is None or image.shape[:2] != (height, width) or image.dtype != np.uint8:
        raise ValueError("invalid_png_pixels")
    if image.ndim == 2:
        image = cv2.cvtColor(image, cv2.COLOR_GRAY2BGR)
    elif image.shape[2] == 4:
        # OCR 背景明确固定为白色；整数合成不分配整幅 F32 图。
        alpha = image[:, :, 3:4].astype(np.uint16)
        image = ((image[:, :, :3].astype(np.uint16) * alpha + 255 * (255 - alpha) + 127) // 255).astype(np.uint8)
    if image.ndim != 3 or image.shape[2] != 3:
        raise ValueError("invalid_png_channels")
    return image


def session(path, expected_inputs, output_name):
    options = ort.SessionOptions()
    options.intra_op_num_threads = 2; options.inter_op_num_threads = 1
    options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    options.add_session_config_entry("session.intra_op.allow_spinning", "0")
    options.add_session_config_entry("session.inter_op.allow_spinning", "0")
    model = ort.InferenceSession(path, sess_options=options, providers=["CPUExecutionProvider"])
    actual = {item.name: item for item in model.get_inputs()}
    if set(actual) != set(expected_inputs) or len(model.get_outputs()) != 1 or model.get_outputs()[0].name != output_name or model.get_outputs()[0].type != "tensor(float)":
        raise ValueError("model_io_schema")
    for name, (dtype, rank, fixed) in expected_inputs.items():
        item = actual[name]
        if item.type != dtype or len(item.shape) != rank or any(isinstance(item.shape[index], int) and item.shape[index] != size for index, size in fixed.items()):
            raise ValueError("model_io_schema")
    return model


def ordered_quad(points):
    points = np.asarray(points, np.float32).reshape(4, 2)
    left = points[np.argsort(points[:, 0], kind="stable")[:2]]
    right = points[np.argsort(points[:, 0], kind="stable")[2:]]
    left = left[np.argsort(left[:, 1], kind="stable")]; right = right[np.argsort(right[:, 1], kind="stable")]
    return np.array([left[0], right[0], right[1], left[1]], np.float32)


def tile_origins(length, tile=960):
    if length <= tile:
        return [0]
    result = list(range(0, length - tile + 1, 864))
    if result[-1] != length - tile:
        result.append(length - tile)
    return result


def detect_scale(image, model, options, deadline, trace, include_cores=False):
    height, width = image.shape[:2]
    padded_h, padded_w = ((max(height, 128) + 31) // 32) * 32, ((max(width, 128) + 31) // 32) * 32
    border = np.rint(np.array([image[0, 0], image[0, -1], image[-1, 0], image[-1, -1]], np.float32).mean(axis=0)).tolist()
    padded = cv2.copyMakeBorder(image, 0, padded_h - height, 0, padded_w - width, cv2.BORDER_CONSTANT, value=border)
    probability = np.zeros((padded_h, padded_w), np.uint8)
    split = max(padded_h, padded_w) >= 1440
    origins_y = tile_origins(padded_h) if split else [0]
    origins_x = tile_origins(padded_w) if split else [0]
    if len(origins_y) * len(origins_x) > 128:
        raise ValueError("det_tile_budget")
    for y in origins_y:
        for x in origins_x:
            check_deadline(deadline)
            tile = padded[y:y + (960 if split else padded_h), x:x + (960 if split else padded_w)]
            rgb = cv2.cvtColor(tile, cv2.COLOR_BGR2RGB).astype(np.float32) / 255
            tensor = np.ascontiguousarray(((rgb - [.485, .456, .406]) / [.229, .224, .225]).transpose(2, 0, 1)[None], dtype=np.float32)
            output = model.run(["fetch_name_0"], {"x": tensor})[0]
            if output.shape != (1, 1, tile.shape[0], tile.shape[1]) or not np.isfinite(output).all():
                raise ValueError("det_output_schema")
            tile_map = np.rint(output[0, 0].clip(0, 1) * 255).astype(np.uint8)
            probability[y:y + tile.shape[0], x:x + tile.shape[1]] |= tile_map
    probability = probability[:height, :width]
    bitmap = (probability > options["bitmapThreshold"] * 255).astype(np.uint8) * 255
    bitmap = cv2.dilate(bitmap, np.ones((2, 2), np.uint8))
    contours, _ = cv2.findContours(bitmap, cv2.RETR_LIST, cv2.CHAIN_APPROX_SIMPLE)
    if len(contours) > 1000:
        raise ValueError("det_contour_budget")
    quads = []; cores = []
    for contour in contours:
        check_deadline(deadline)
        rectangle = cv2.minAreaRect(contour)
        if min(rectangle[1]) < 3:
            continue
        quad = ordered_quad(cv2.boxPoints(rectangle))
        lo = np.maximum(np.floor(quad.min(axis=0)).astype(int), 0)
        hi = np.minimum(np.ceil(quad.max(axis=0)).astype(int) + 1, [width, height])
        if np.any(hi <= lo):
            continue
        mask = np.zeros((hi[1] - lo[1], hi[0] - lo[0]), np.uint8)
        cv2.fillPoly(mask, [np.rint(quad - lo).astype(np.int32)], 1)
        score = cv2.mean(probability[lo[1]:hi[1], lo[0]:hi[0]], mask)[0] / 255
        if score < options["boxThreshold"]:
            continue
        core = quad.copy()
        core[:, 0] = core[:, 0].clip(0, width); core[:, 1] = core[:, 1].clip(0, height)
        perimeter = cv2.arcLength(quad, True)
        if perimeter <= 0:
            continue
        offset = abs(cv2.contourArea(quad)) * options["unclipRatio"] / perimeter
        clipper = pyclipper.PyclipperOffset()
        clipper.AddPath(np.rint(quad * 1024).astype(np.int64).tolist(), pyclipper.JT_ROUND, pyclipper.ET_CLOSEDPOLYGON)
        expanded = clipper.Execute(offset * 1024)
        if len(expanded) != 1:
            continue
        rectangle = cv2.minAreaRect(np.asarray(expanded[0], np.float32) / 1024)
        if min(rectangle[1]) < 5:
            continue
        quad = ordered_quad(cv2.boxPoints(rectangle))
        quad[:, 0] = quad[:, 0].clip(0, width); quad[:, 1] = quad[:, 1].clip(0, height)
        if cv2.contourArea(quad) > 1:
            quads.append(quad); cores.append(core)
    if len(quads) > MAX_NODES:
        raise ValueError("layout_node_budget")
    # 明确的 Clippy 初始 ID 顺序，避免 contour 遍历顺序主导结果。
    order = sorted(range(len(quads)), key=lambda i: (float(quads[i][:, 1].mean()), float(quads[i][:, 0].min())))
    quads, cores = [quads[i] for i in order], [cores[i] for i in order]
    trace("det", {"tiles": len(origins_y) * len(origins_x), "quads": [quad.tolist() for quad in quads], "coreQuads": [quad.tolist() for quad in cores]})
    return (quads, cores) if include_cores else quads



def reconcile_overview(details, overview, detail_cores=None):
    """Clippy 多尺度框协调：完整低分辨率行替换同一行的接缝碎片，仍从原图识别。"""
    remaining = list(details)
    selected = []
    for whole in overview:
        edges = np.roll(whole, -1, axis=0) - whole
        lengths = np.linalg.norm(edges, axis=1)
        short = float(lengths.min())
        direction = edges[int(lengths.argmax())] / max(float(lengths.max()), 1e-6)
        normal = np.array([-direction[1], direction[0]], np.float32)
        whole_axis = whole @ direction
        matches = []
        spans = []
        for index, part in enumerate(remaining):
            if part is None:
                continue
            part_edges = np.roll(part, -1, axis=0) - part
            part_lengths = np.linalg.norm(part_edges, axis=1)
            part_direction = part_edges[int(part_lengths.argmax())] / max(float(part_lengths.max()), 1e-6)
            # 高度/方向约束防止 overview 的跨行或跨列误框吞掉清晰 detail 行。
            if not .6 <= float(part_lengths.min()) / max(short, 1e-6) <= 1.7 or abs(float(direction @ part_direction)) < .95:
                continue
            area = abs(cv2.contourArea(part))
            intersection, _ = cv2.intersectConvexConvex(whole.astype(np.float32), part.astype(np.float32))
            # 行轴完整性用unclip前的文字core判断；扩框留白不是实际丢失的字符。
            core = detail_cores[index] if detail_cores is not None else part
            part_axis = core @ direction
            overlap = max(0.0, min(float(whole_axis.max()), float(part_axis.max())) - max(float(whole_axis.min()), float(part_axis.min())))
            axis_coverage = overlap / max(float(np.ptp(part_axis)), 1e-6)
            normal_distance = abs(float((part.mean(0) - whole.mean(0)) @ normal))
            # 接缝可把 detail 框的上下留白膨胀。用同一法向中心+轴向覆盖证明同一行，
            # 不把面积阈值普遍调低；临近小字仍受自身行高的中心容差保护。
            same_centerline = normal_distance <= .25 * min(short, float(part_lengths.min()))
            # overview 对小字可能只检测中段；不允许较短框截断已有完整 detail。
            if axis_coverage >= .95 and (intersection / max(area, 1e-6) >= .72 or same_centerline):
                matches.append(index)
                spans.append((float(part_axis.min()), float(part_axis.max())))
        # overview 若跨越 detail 已明确区分的宽列间隙，保留 detail，不能吞并两列。
        spans.sort()
        end = spans[0][1] if spans else 0
        column_gap = False
        for start, stop in spans[1:]:
            if start - end > 1.5 * short:
                column_gap = True
            end = max(end, stop)
        # 有 detail 支持才用 overview，不能用单独弱尺度幻觉增加新行。
        if matches and not column_gap:
            for index in matches:
                remaining[index] = None
            selected.append(whole)
    return [quad for quad in remaining if quad is not None] + selected


def remove_contained_quads(quads):
    """RETR_LIST 的字形内洞可能成为小框；只去掉几乎完全被大行框包含的内框。"""
    areas = [abs(cv2.contourArea(quad)) for quad in quads]
    result = []
    for index, quad in enumerate(quads):
        contained = False
        for other, outer in enumerate(quads):
            if index == other or areas[index] > .2 * areas[other]:
                continue
            overlap, _ = cv2.intersectConvexConvex(quad.astype(np.float32), outer.astype(np.float32))
            if overlap / max(areas[index], 1e-6) >= .98:
                contained = True
                break
        if not contained:
            result.append(quad)
    return result


def detect(image, model, options, deadline, trace):
    details, cores = detect_scale(image, model, options, deadline, trace, include_cores=True)
    height, width = image.shape[:2]
    if max(width, height) >= 2880:
        # 单次 1280 边长 overview 提供跨 960 tile 的上下文；保留未覆盖的小字 detail。
        scale = 1280 / max(width, height)
        size = (max(1, round(width * scale)), max(1, round(height * scale)))
        overview_image = cv2.resize(image, size, interpolation=cv2.INTER_AREA)
        overview = detect_scale(overview_image, model, options, deadline, lambda stage, data: trace("overview_" + stage, data))
        factors = np.array([width / size[0], height / size[1]], np.float32)
        overview = [quad * factors for quad in overview]
        result = remove_contained_quads(reconcile_overview(details, overview, cores))
        if len(result) > MAX_NODES:
            raise ValueError("layout_node_budget")
        trace("multiscale", {"schema": "clippy-overview-detail-v1", "detailCount": len(details), "overviewCount": len(overview), "quads": [quad.tolist() for quad in result]})
    else:
        result = remove_contained_quads(details)
    result.sort(key=lambda quad: (float(quad[:, 1].mean()), float(quad[:, 0].min())))
    return result


def crop_line(image, quad):
    width = max(np.linalg.norm(quad[1] - quad[0]), np.linalg.norm(quad[2] - quad[3]))
    height = max(np.linalg.norm(quad[3] - quad[0]), np.linalg.norm(quad[2] - quad[1]))
    if not math.isfinite(width + height) or min(width, height) < 1:
        raise ValueError("rec_degenerate_crop")
    rotate = height >= 1.5 * width
    ratio = height / width if rotate else width / height
    target_w = max(1, int(math.floor(48 * ratio + .5)))
    if target_w > MAX_REC_WIDTH:
        raise ValueError("rec_width_budget")
    # 先在原图坐标作透视，再缩至H48，避免缩放后的DOM坐标和不等比拉伸。
    pixel_w, pixel_h = max(1, int(round(width))), max(1, int(round(height)))
    if pixel_w * pixel_h > MAX_PIXELS:
        raise ValueError("rec_crop_budget")
    destination = np.array([[0, 0], [pixel_w - 1, 0], [pixel_w - 1, pixel_h - 1], [0, pixel_h - 1]], np.float32)
    transformed = cv2.warpPerspective(image, cv2.getPerspectiveTransform(quad.astype(np.float32), destination), (pixel_w, pixel_h), flags=cv2.INTER_CUBIC, borderMode=cv2.BORDER_REPLICATE)
    if rotate:
        transformed = cv2.rotate(transformed, cv2.ROTATE_90_COUNTERCLOCKWISE)
    return cv2.resize(transformed, (target_w, 48), interpolation=cv2.INTER_CUBIC)


def decode_ctc(output, dictionary):
    if output.ndim != 3 or output.shape[0] != 1 or output.shape[2] != len(dictionary) or not 1 <= output.shape[1] <= 2048 or not np.isfinite(output).all():
        raise ValueError("rec_output_schema")
    probabilities = output[0]
    if probabilities.min() < -.00001 or probabilities.max() > 1.00001:
        raise ValueError("rec_output_probability")
    indices = probabilities.argmax(axis=1)
    previous = -1; characters = []; confidence = []
    for step, index in enumerate(indices):
        if index != 0 and index != previous:
            characters.append(dictionary[index]); confidence.append(float(probabilities[step, index]))
        previous = index
    return "".join(characters), confidence


def recognize(png, manifest, deadline, trace=lambda _stage, _data: None):
    cv2.setNumThreads(1)
    options = validate_manifest(manifest); check_deadline(deadline)
    image = decode_png(png); height, width = image.shape[:2]
    dictionary = [""] + Path(manifest["models"]["dictionary"]["path"]).read_text(encoding="utf-8").splitlines() + [" "]
    if len(dictionary) != 18710:
        raise ValueError("dictionary_class_count")
    det = session(manifest["models"]["det"]["path"], {"x": ("tensor(float)", 4, {0: 1, 1: 3})}, "fetch_name_0")
    check_deadline(deadline)
    edge = session(manifest["models"]["edge"]["path"], {"node_features": ("tensor(float)", 2, {1: 3}), "edge_index": ("tensor(int64)", 2, {1: 2}), "base_edge_features": ("tensor(float)", 2, {1: 2}), "adv_edge_features": ("tensor(float)", 2, {1: 17})}, "edge_logits")
    check_deadline(deadline)
    rec = session(manifest["models"]["rec"]["path"], {"x": ("tensor(float)", 4, {0: 1, 1: 3, 2: 48})}, "fetch_name_0")
    check_deadline(deadline)
    quads = detect(image, det, options, deadline, trace)
    inputs, geometry = build_features(image, quads); check_deadline(deadline)
    executed = len(inputs["edge_index"]) > 0
    if executed:
        logits = edge.run(["edge_logits"], inputs)[0]
        model_groups = group_lines(inputs, logits, geometry, options["layoutThreshold"])
        groups = merge_visual_paragraphs(model_groups, geometry)
        trace("edge", {"shapes": {key: list(value.shape) for key, value in inputs.items()}, "hashes": {key: hashlib.sha256(value.tobytes()).hexdigest() for key, value in inputs.items()}, "logits": np.asarray(logits).tolist(), "modelGroups": model_groups, "groups": groups})
    else:
        groups = [[index] for index in range(len(quads))]
        trace("edge_skipped", {"nodes": len(quads), "reason": "no_edges"})
    check_deadline(deadline)
    lines = []; paragraphs = []; total_crop_pixels = 0
    for paragraph_id, group in enumerate(groups):
        ids = []
        for node in group:
            check_deadline(deadline)
            crop = crop_line(image, quads[node]); total_crop_pixels += crop.shape[0] * crop.shape[1]
            if total_crop_pixels > MAX_REC_PIXELS:
                raise ValueError("rec_total_budget")
            tensor = np.ascontiguousarray((cv2.cvtColor(crop, cv2.COLOR_BGR2RGB).astype(np.float32) / 255 - .5).transpose(2, 0, 1)[None])
            output = rec.run(["fetch_name_0"], {"x": tensor})[0]
            text, confidences = decode_ctc(output, dictionary)
            mean = float(np.mean(confidences)) if confidences else 0.0
            line = {"id": int(node), "quad": quads[node].astype(float).tolist(), "text": text, "confidence": mean,
                    "accepted": bool(confidences) and mean >= options["lineThreshold"], "charConfidences": confidences, "paragraphId": paragraph_id, "readingOrder": len(lines)}
            lines.append(line); ids.append(int(node))
            trace("rec", {"node": int(node), "inputShape": list(tensor.shape), "outputShape": list(output.shape), "text": text, "confidence": mean})
            del output, tensor
        paragraphs.append({"id": paragraph_id, "lineIds": ids, "readingOrder": paragraph_id})
    texts = {line["id"]: line["text"] if line["accepted"] else "" for line in lines}
    paragraph_texts = ["\n".join(texts[node] for node in paragraph["lineIds"] if texts[node]) for paragraph in paragraphs]
    return {"width": width, "height": height, "text": "\n\n".join(text for text in paragraph_texts if text), "lines": lines,
            "paragraphs": paragraphs, "pipeline": {"id": manifest["pipelineId"], "engine": "ppocrv6+edgegnn", "featureSchema": SCHEMA,
            "layoutExecuted": executed, "layoutReason": None if executed else ("no_text" if not quads else "single_line")}, "fallbackReason": None}
