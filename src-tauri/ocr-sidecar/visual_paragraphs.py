"""不依赖模型运行时的保守视觉段落合并。"""


def merge_visual_paragraphs(groups, geometry):
    """合并同栏正常行距的相邻 GNN 片段，保留大段距、跨栏与标题边界。"""
    if len(groups) < 2:
        return groups
    line_heights = sorted(
        max(float(item["max"][1] - item["min"][1]), 1.0) for item in geometry
    )
    median_height = line_heights[(len(line_heights) - 1) // 2]

    def bounds(nodes):
        minimum = [
            min(float(geometry[node]["min"][axis]) for node in nodes)
            for axis in range(2)
        ]
        maximum = [
            max(float(geometry[node]["max"][axis]) for node in nodes)
            for axis in range(2)
        ]
        return minimum, maximum

    def median_node_height(nodes):
        heights = sorted(
            max(
                float(geometry[node]["max"][1] - geometry[node]["min"][1]),
                1.0,
            )
            for node in nodes
        )
        return heights[(len(heights) - 1) // 2]

    merged = [list(groups[0])]
    for current in groups[1:]:
        previous = merged[-1]
        previous_min, previous_max = bounds(previous)
        current_min, current_max = bounds(current)
        previous_width = max(previous_max[0] - previous_min[0], 1.0)
        current_width = max(current_max[0] - current_min[0], 1.0)
        overlap = max(
            0.0,
            min(previous_max[0], current_max[0])
            - max(previous_min[0], current_min[0]),
        )
        overlap_ratio = overlap / min(previous_width, current_width)
        vertical_gap = current_min[1] - previous_max[1]
        previous_height = median_node_height(previous)
        current_height = median_node_height(current)
        height_ratio = max(previous_height, current_height) / min(
            previous_height, current_height
        )
        left_delta = abs(current_min[0] - previous_min[0])
        same_visual_block = (
            -0.15 * median_height <= vertical_gap <= median_height
            and overlap_ratio >= 0.25
            and height_ratio <= 1.8
            and (left_delta <= 1.5 * median_height or overlap_ratio >= 0.6)
        )
        if same_visual_block:
            previous.extend(current)
        else:
            merged.append(list(current))
    return merged
