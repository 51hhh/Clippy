"""真实模型分数的受约束 DSU；分组与阅读顺序分别计算。"""
import math
import numpy as np


def order_groups(groups, geometry):
    """Clippy XY-cut 顺序：先贯通列空隙，再区段；基线容差不改变GNN组。"""
    if len(groups) < 2:
        return groups
    heights = sorted(max(float(item["max"][1] - item["min"][1]), 1) for item in geometry)
    height = heights[(len(heights) - 1) // 2]
    bounds = [(np.min([geometry[i]["min"] for i in group], axis=0), np.max([geometry[i]["max"] for i in group], axis=0)) for group in groups]

    def ordered(indices):
        if len(indices) < 2:
            return indices
        for axis, required_gap in [(0, height * 1.5), (1, height * .8)]:
            sequence = sorted(indices, key=lambda index: float(bounds[index][0][axis]))
            frontier = float(bounds[sequence[0]][1][axis]); cuts = []
            for at in range(1, len(sequence)):
                index = sequence[at]
                gap = float(bounds[index][0][axis]) - frontier
                if gap > required_gap:
                    cuts.append((gap, at))
                frontier = max(frontier, float(bounds[index][1][axis]))
            if cuts:
                _, at = max(cuts)
                return ordered(sequence[:at]) + ordered(sequence[at:])
        # 没有贯通gutter时按行阅读。相差不足半个行高的组视为同基线，左到右。
        pending = sorted(indices, key=lambda index: float(bounds[index][0][1]))
        result = []
        while pending:
            top = float(bounds[pending[0]][0][1])
            row = [index for index in pending if float(bounds[index][0][1]) - top <= height * .5]
            row.sort(key=lambda index: (float(bounds[index][0][0]), index))
            result.extend(row)
            pending = [index for index in pending if index not in row]
        return result
    return [groups[index] for index in ordered(list(range(len(groups))))]


def group_lines(inputs, logits, geometry, threshold=.52):
    n = len(geometry)
    logits = np.asarray(logits)
    if logits.shape != (len(inputs["edge_index"]),) or not np.isfinite(logits).all():
        raise ValueError("layout_invalid_output")
    pairs = {}
    for index, (source, target) in enumerate(inputs["edge_index"]):
        key = tuple(sorted((int(source), int(target))))
        pair = pairs.setdefault(key, {"probs": [], "base": [], "adv": []})
        score = float(logits[index])
        # 稳定 sigmoid，模型 manifest 明确是 raw logit。
        probability = 1 / (1 + math.exp(-score)) if score >= 0 else math.exp(score) / (1 + math.exp(score))
        pair["probs"].append(probability); pair["base"].append(inputs["base_edge_features"][index]); pair["adv"].append(inputs["adv_edge_features"][index])
    for pair in pairs.values():
        b, a = np.array(pair["base"]), np.array(pair["adv"])
        pair["mean"] = float(np.mean(pair["probs"])); pair["max"] = max(pair["probs"])
        pair["compatible"] = bool(a[:, 8].max() >= .5 or b[:, 1].mean() >= .35 or (
            b[:, 1].mean() >= .15 and a[:, 14].mean() <= .7884574 and abs(a[:, 2]).mean() <= 1.60
            and a[:, 3].mean() <= 1.50 and a[:, 5].mean() <= 1.75 and a[:, 7].mean() <= 2.50))
    parents = list(range(n)); sizes = [1] * n

    def find(node):
        while parents[node] != node:
            parents[node] = parents[parents[node]]; node = parents[node]
        return node

    for (i, j), pair in sorted(pairs.items(), key=lambda item: (-item[1]["mean"], -item[1]["max"], item[0])):
        ri, rj = find(i), find(j)
        if ri == rj:
            continue
        t08, t10, t12, tm3 = min(threshold + .08, .99), min(threshold + .10, .99), min(threshold + .12, .99), max(threshold - .03, 0)
        reciprocal = len(pair["probs"]) >= 2
        accept = pair["mean"] >= threshold and min(pair["probs"]) >= tm3 if reciprocal else pair["probs"][0] >= t08 and pair["compatible"]
        if accept and sizes[ri] >= 2 and sizes[rj] >= 2:
            cross = [p for (a, b), p in pairs.items() if {find(a), find(b)} == {ri, rj}]
            accept = (pair["mean"] >= t08 if reciprocal else pair["probs"][0] >= t12) and len(cross) >= 3 and (
                any(p["mean"] >= threshold and p["compatible"] for p in cross) or max(p["max"] for p in cross) >= t10)
        if accept:
            if sizes[ri] < sizes[rj]:
                ri, rj = rj, ri
            parents[rj] = ri; sizes[ri] += sizes[rj]
    grouped = {}
    for node in range(n):
        grouped.setdefault(find(node), []).append(node)
    rank = {node: index for index, node in enumerate(sorted(range(n), key=lambda i: (geometry[i]["center"][1], geometry[i]["min"][0], geometry[i]["center"][0], i)))}
    groups = [sorted(nodes, key=rank.get) for _, nodes in sorted(grouped.items())]
    bounds = [(np.min([geometry[i]["min"] for i in nodes], axis=0), np.max([geometry[i]["max"] for i in nodes], axis=0)) for nodes in groups]
    fragments = []
    for index, nodes in enumerate(groups):
        lo, hi = bounds[index]
        intersecting = []
        for other, (blo, bhi) in enumerate(bounds):
            if other == index:
                continue
            overlap = np.minimum(hi, bhi) - np.maximum(lo, blo)
            if overlap[0] / max(min(hi[0] - lo[0], bhi[0] - blo[0]), 1e-6) >= .03 and overlap[1] > 1e-6:
                intersecting.extend(groups[other])
        fragment = []
        for node in nodes:
            if fragment and any(rank[fragment[-1]] < rank[v] < rank[node] for v in intersecting):
                fragments.append(fragment); fragment = []
            fragment.append(node)
        if fragment:
            fragments.append(fragment)
    return order_groups(fragments, geometry)
