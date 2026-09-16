"""Clippy v1 图特征：明确的方向/尺度策略，非私有实现兼容 ABI。"""
import math
import cv2
import numpy as np

SCHEMA = "clippy-edge-features-v1"
EPS = 1e-6
MAX_NODES = 512


def lower_median(values, default=0.0):
    return float(sorted(values)[(len(values) - 1) // 2]) if len(values) else default


def color_features(bgr, quad):
    height, width = bgr.shape[:2]
    lo = np.maximum(np.floor(quad.min(axis=0)).astype(int), 0)
    hi = np.minimum(np.ceil(quad.max(axis=0)).astype(int) + 1, [width, height])
    if np.any(hi <= lo):
        return np.zeros(3), np.zeros(3)
    crop = bgr[lo[1]:hi[1], lo[0]:hi[0]]
    mask = np.zeros(crop.shape[:2], np.uint8)
    cv2.fillPoly(mask, [np.rint(quad - lo).astype(np.int32)], 255)
    pixels = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)[mask != 0]
    if not len(pixels):
        return np.zeros(3), np.zeros(3)
    threshold, binary = cv2.threshold(pixels.reshape(-1, 1), 0, 255, cv2.THRESH_BINARY | cv2.THRESH_OTSU)
    minority = 255 if np.count_nonzero(binary) < len(pixels) / 2 else 0
    gray = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
    classes = np.where(gray > threshold, 255, 0)

    def encode(value):
        selected = (mask != 0) & (classes == value)
        if not selected.any():
            return np.zeros(3)
        # OpenCV 8-bit HSV；先求 BGR 均值，再转换，避免色相平均错误。
        mean = np.rint(crop[selected].mean(axis=0)).clip(0, 255).astype(np.uint8).reshape(1, 1, 3)
        h, s, v = cv2.cvtColor(mean, cv2.COLOR_BGR2HSV)[0, 0].astype(float)
        return np.array([math.cos(2 * math.pi * h / 180) * s / 255,
                         math.sin(2 * math.pi * h / 180) * s / 255, v / 255])
    return encode(minority), encode(255 - minority)


def build_features(bgr, quads):
    quads = np.asarray(quads, np.float64).reshape(-1, 4, 2)
    n = len(quads)
    if n > MAX_NODES or not np.isfinite(quads).all():
        raise ValueError("layout_input_budget")
    if n == 0:
        return {"node_features": np.empty((0, 3), np.float32), "edge_index": np.empty((0, 2), np.int64),
                "base_edge_features": np.empty((0, 2), np.float32), "adv_edge_features": np.empty((0, 17), np.float32)}, []
    vectors = np.roll(quads, -1, axis=1) - quads
    lengths = np.linalg.norm(vectors, axis=2)
    longest, shortest = lengths.max(axis=1), lengths.min(axis=1)
    if np.any(shortest <= EPS):
        raise ValueError("layout_degenerate_quad")
    direction_vectors = vectors[np.arange(n), lengths.argmax(axis=1)]
    angles = (np.arctan2(direction_vectors[:, 1], direction_vectors[:, 0]) + math.pi / 2) % math.pi - math.pi / 2
    # v1 明确采用角度 lower median / 短边 lower median，完整原统计未恢复。
    angle = lower_median(angles)
    u, v = math.cos(angle), math.sin(angle)
    scale = 32.0 / max(lower_median(shortest), EPS)
    canonical = quads @ np.array([[u, -v], [v, u]]) * scale
    minimum, maximum = canonical.min(axis=1), canonical.max(axis=1)
    centers = (minimum + maximum) / 2
    widths = maximum[:, 0] - minimum[:, 0]
    heights = np.maximum(shortest * scale, EPS)
    ell = np.log1p(np.maximum(longest * scale, EPS) / 32)
    colors = [color_features(bgr, quad) for quad in quads]
    a = np.array([pair[0] for pair in colors]); b = np.array([pair[1] for pair in colors])
    dx = centers[None, :, 0] - centers[:, None, 0]
    dy = centers[None, :, 1] - centers[:, None, 1]
    distance = np.hypot(dx, dy)
    overlap = np.maximum(0, np.minimum(maximum[:, None, 0], maximum[None, :, 0]) - np.maximum(minimum[:, None, 0], minimum[None, :, 0]))
    rx = overlap / np.maximum(np.minimum(widths[:, None], widths[None, :]), EPS)
    hbar = np.maximum((heights[:, None] + heights[None, :]) / 2, EPS)
    gap = np.where(dy >= 0, minimum[None, :, 1] - maximum[:, None, 1], maximum[None, :, 1] - minimum[:, None, 1])
    nearest = [min((j for j in range(n) if j != i), key=lambda j: (rx[i, j] <= .1, abs(dy[i, j]), abs(dx[i, j]), j), default=-1) for i in range(n)]
    gaps = [abs(gap[i, j]) for i, j in enumerate(nearest) if j >= 0 and abs(gap[i, j]) > EPS]
    mu = lower_median(gaps, 32.0)
    sigma = max(lower_median([abs(value - mu) for value in gaps]), 8.0)
    psi = np.zeros((n, 5), np.float64)
    for i in range(n):
        neighbors = [j for j in range(n) if j != i and rx[i, j] >= .15 and abs(dy[i, j]) <= 7 * hbar[i, j]]
        psi[i, 0] = (minimum[i, 0] - lower_median([minimum[j, 0] for j in neighbors], minimum[i, 0])) / heights[i]
        psi[i, 1] = (maximum[i, 0] - lower_median([maximum[j, 0] for j in neighbors], maximum[i, 0])) / heights[i]
        neighbor_widths = sorted(widths[j] for j in neighbors)
        lo, hi = np.searchsorted(neighbor_widths, widths[i], side="left"), np.searchsorted(neighbor_widths, widths[i], side="right")
        psi[i, 2] = .5 if len(neighbors) < 2 else (lo + max(lo, hi - 1)) / (2 * max(len(neighbors) - 1, 1))
        psi[i, 3] = sum(abs(abs(gap[i, j]) - mu) / sigma <= 2.5 and abs(gap[i, j]) / hbar[i, j] <= 1.75 for j in neighbors)
        below = [abs(gap[i, j]) / hbar[i, j] for j in neighbors if dy[i, j] > 0]
        above = [abs(gap[i, j]) / hbar[i, j] for j in neighbors if dy[i, j] < 0]
        psi[i, 4] = min(below, default=mu / heights[i]) - min(above, default=mu / heights[i])
    edges, base, advanced = [], [], []
    for i in range(n):
        others = [j for j in range(n) if j != i]
        candidates = set(sorted(others, key=lambda j: (distance[i, j], j))[:11])
        for sign in [-1, 1]:
            choices = [j for j in others if sign * dy[i, j] > 0]
            if choices:
                candidates.add(min(choices, key=lambda j: (abs(dy[i, j]), distance[i, j], j)))
        choices = [j for j in others if rx[i, j] > 0]
        if choices:
            candidates.add(min(choices, key=lambda j: (-rx[i, j], distance[i, j], j)))
        if others:
            candidates.add(min(others, key=lambda j: (abs(minimum[j, 0] - minimum[i, 0]), abs(dy[i, j]), distance[i, j], j)))
        for j in sorted(candidates, key=lambda j: (distance[i, j], j)):
            h = hbar[i, j]
            edges.append([i, j]); base.append([abs(ell[i] - ell[j]), rx[i, j]])
            advanced.append([dy[i, j] / h, gap[i, j] / h, dx[i, j] / h, abs(gap[i, j]) / h,
                abs(minimum[i, 0] - minimum[j, 0]) / h, abs(dx[i, j]) / h, abs(maximum[i, 0] - maximum[j, 0]) / h,
                abs(abs(gap[i, j]) - mu) / sigma, float(nearest[i] == j and nearest[j] == i),
                abs(psi[i, 0] - psi[j, 0]), abs(psi[i, 1] - psi[j, 1]), abs(psi[i, 2] - psi[j, 2]),
                min(psi[i, 3], psi[j, 3]) / max(psi[i, 3], psi[j, 3], EPS), abs(psi[i, 4] - psi[j, 4]),
                abs(math.log(max(heights[i], heights[j]) / min(heights[i], heights[j]))),
                float(np.linalg.norm(a[i] - a[j])), float(np.linalg.norm(b[i] - b[j]))])
    inputs = {"node_features": np.column_stack((ell, shortest / longest, a[:, 2])).astype(np.float32),
              "edge_index": np.asarray(edges, np.int64).reshape(-1, 2), "base_edge_features": np.asarray(base, np.float32).reshape(-1, 2),
              "adv_edge_features": np.asarray(advanced, np.float32).reshape(-1, 17)}
    if any(not np.isfinite(value).all() for value in inputs.values()):
        raise ValueError("layout_nonfinite_features")
    geometry = [{"min": minimum[i], "max": maximum[i], "center": centers[i]} for i in range(n)]
    return inputs, geometry
