"""纯合成公式/协议回归；真实模型验收由独立fixture另外完成。"""
import math
from pathlib import Path
import sys
import unittest

import numpy as np
sys.path.insert(0, str(Path(__file__).resolve().parent))
from edge_features import build_features, color_features
from layout_groups import group_lines, order_groups
from pipeline import decode_ctc, decode_ctc_detailed, decode_png, tile_origins, reconcile_overview, remove_contained_quads


def quad(x, y, width=100, height=20):
    return np.array([[x, y], [x + width, y], [x + width, y + height], [x, y + height]], np.float32)


class FeatureTests(unittest.TestCase):
    def test_all_columns_two_equal_lines_and_reverse_direction(self):
        inputs, geometry = build_features(np.full((80, 120, 3), 255, np.uint8), [quad(0, 0), quad(0, 30)])
        np.testing.assert_allclose(inputs["node_features"], [[math.log(6), .2, 0]] * 2, rtol=1e-6)
        np.testing.assert_array_equal(inputs["edge_index"], [[0, 1], [1, 0]])
        np.testing.assert_allclose(inputs["base_edge_features"], [[0, 1], [0, 1]])
        forward = [1.5, .5, 0, .5, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0]
        reverse = [-1.5, -.5, 0, .5, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0]
        np.testing.assert_allclose(inputs["adv_edge_features"], [forward, reverse], atol=1e-6)
        self.assertEqual(group_lines(inputs, np.array([5, 5], np.float32), geometry), [[0, 1]])
        self.assertEqual(group_lines(inputs, np.array([-5, -5], np.float32), geometry), [[0], [1]])

    def test_sparse_candidate_limit_and_finite_empty_singleton(self):
        image = np.full((500, 500, 3), 255, np.uint8)
        inputs, _ = build_features(image, [quad((i % 4) * 110, (i // 4) * 30) for i in range(16)])
        edges = inputs["edge_index"]
        self.assertLessEqual(len(edges), 16 * 15)
        self.assertEqual(len(set(map(tuple, edges))), len(edges))
        self.assertTrue(np.all(edges[:, 0] != edges[:, 1]))
        for value in inputs.values():
            self.assertTrue(np.isfinite(value).all())
        for quads in [[], [quad(0, 0)]]:
            result, _ = build_features(image, quads)
            self.assertEqual(result["edge_index"].shape, (0, 2))
            self.assertEqual(result["adv_edge_features"].shape, (0, 17))

    def test_minority_color_and_tie_rule(self):
        image = np.full((20, 100, 3), 255, np.uint8); image[:10] = 0
        a, b = color_features(image, quad(0, 0, 99, 19))
        np.testing.assert_array_equal(a, [0, 0, 0]); np.testing.assert_array_equal(b, [0, 0, 1])

    def test_columns_and_spanning_title_preserve_group_membership(self):
        quads = [quad(300, 0), quad(0, 1), quad(300, 35), quad(0, 36), quad(0, -50, 400)]
        geometry = [{"min": value.min(0), "max": value.max(0), "center": value.mean(0)} for value in quads]
        self.assertEqual(order_groups([[0], [1], [2], [3]], geometry), [[1], [3], [0], [2]])
        self.assertEqual(order_groups([[0, 2], [1, 3], [4]], geometry), [[4], [1, 3], [0, 2]])

    def test_overview_replaces_seam_fragments_without_swallowing_other_rows(self):
        parts = [quad(0, 0, 80), quad(75, 0, 200), quad(0, 40, 60), quad(350, 0, 60)]
        whole = quad(0, 0, 280)
        result = reconcile_overview(parts, [whole, quad(0, 0, 280, 80)])
        self.assertEqual(len(result), 3)
        for expected in [parts[2], parts[3], whole]:
            self.assertTrue(any(np.array_equal(item, expected) for item in result))
        # 完全无 detail 支持的 overview 不添加虚构文字行。
        self.assertEqual(reconcile_overview([], [whole]), [])

    def test_overview_centerline_handles_padding_but_preserves_small_nearby_text_and_columns(self):
        # 完整框与碎片同基线，碎片上下留白较宽；下一行细字不属于这个基线。
        whole = quad(0, 10, 300, 20)
        details = [quad(0, 10, 50, 20), quad(45, 5, 255, 30), quad(30, 32, 80, 8)]
        result = reconcile_overview(details, [whole])
        self.assertEqual(len(result), 2)
        self.assertTrue(any(np.array_equal(item, details[2]) for item in result))
        self.assertTrue(any(np.array_equal(item, whole) for item in result))
        columns = [quad(0, 0, 80), quad(200, 0, 80)]
        result = reconcile_overview(columns, [quad(0, 0, 280)])
        self.assertEqual(len(result), 2)
        np.testing.assert_array_equal(result, columns)

    def test_partial_overview_never_truncates_complete_small_print(self):
        detail = quad(700, 480, 650, 35)
        partial = quad(740, 465, 500, 52)
        result = reconcile_overview([detail], [partial])
        np.testing.assert_array_equal(result, [detail])

    def test_core_coverage_distinguishes_word_padding_from_truncated_small_print(self):
        for scale in [.5, 1, 6, 10]:
            whole = quad(0, 0, 300, 20) * scale
            word = quad(-8, -4, 68, 28) * scale
            core = quad(2, 1, 48, 18) * scale
            result = reconcile_overview([word], [whole], [core])
            np.testing.assert_array_equal(result, [whole])
            small = quad(0, 50, 100, 8) * scale
            small_core = quad(1, 51, 98, 6) * scale
            partial = quad(10, 48, 80, 12) * scale
            result = reconcile_overview([small], [partial], [small_core])
            np.testing.assert_array_equal(result, [small])

    def test_contained_letter_holes_do_not_erase_adjacent_small_text(self):
        line = quad(100, 100, 600, 120)
        hole = quad(650, 185, 12, 16)
        neighbor = quad(100, 222, 300, 20)
        result = remove_contained_quads([line, hole, neighbor])
        self.assertEqual(len(result), 2)
        np.testing.assert_array_equal(result[0],line)
        np.testing.assert_array_equal(result[1],neighbor)

    def test_ctc_blank_resets_repeat_and_preserves_space(self):
        output = np.zeros((1, 7, 4), np.float32)
        for i, index in enumerate([1, 1, 0, 1, 3, 2, 2]):
            output[0, i, index] = .9
        text, confidences = decode_ctc(output, ["", "A", "B", " "])
        self.assertEqual(text, "AA B"); self.assertEqual(len(confidences), 4)
        output[0, 0, 0] = float("nan")
        with self.assertRaisesRegex(ValueError, "schema"):
            decode_ctc(output, ["", "A", "B", " "])

    def test_ctc_diagnostics_keep_emission_steps_blank_runs_and_class_identity(self):
        output = np.zeros((1, 7, 4), np.float32)
        for step, index in enumerate([1, 1, 0, 1, 3, 2, 2]):
            output[0, step, index] = .9
        text, confidences, emissions = decode_ctc_detailed(output, ["", "A", "B", " "])
        self.assertEqual(text, "AA B")
        np.testing.assert_allclose(confidences, [.9] * 4)
        self.assertEqual(
            [{key: item[key] for key in ["step", "classIndex", "character", "blankBefore", "stepGap"]} for item in emissions],
            [
                {"step": 0, "classIndex": 1, "character": "A", "blankBefore": 0, "stepGap": None},
                {"step": 3, "classIndex": 1, "character": "A", "blankBefore": 1, "stepGap": 3},
                {"step": 4, "classIndex": 3, "character": " ", "blankBefore": 0, "stepGap": 1},
                {"step": 5, "classIndex": 2, "character": "B", "blankBefore": 0, "stepGap": 1},
            ],
        )

    def test_tile_end_alignment_and_png_dimension_budget(self):
        self.assertEqual(tile_origins(1000), [0, 40])
        self.assertEqual(tile_origins(960), [0])
        png = b"\x89PNG\r\n\x1a\n" + b"\x00\x00\x00\rIHDR" + (20000).to_bytes(4, "big") + (1).to_bytes(4, "big") + bytes(9)
        with self.assertRaisesRegex(ValueError, "image_budget"):
            decode_png(png)


if __name__ == "__main__":
    unittest.main()
