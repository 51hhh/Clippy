"""纯合成公式/协议回归；真实模型验收由独立fixture另外完成。"""
import math
from pathlib import Path
import sys
import unittest

import numpy as np
sys.path.insert(0, str(Path(__file__).resolve().parent))
from edge_features import build_features, color_features
from layout_groups import baseline_order, group_lines, order_groups, table_like_rows
from pipeline import crop_line, decode_ctc, decode_ctc_detailed, decode_png, model_contract, orient_line, should_use_english_spacing, tile_origins, reconcile_overview, remove_contained_quads


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

    def test_same_baseline_uses_left_to_right_despite_vertical_detection_jitter(self):
        geometry = [
            {"min": np.array([134.4, 158.4]), "max": np.array([206.6, 180.6])},
            {"min": np.array([27.4, 156.4]), "max": np.array([114.6, 182.6])},
            {"min": np.array([219.3, 158.3]), "max": np.array([301.7, 180.7])},
            {"min": np.array([25.0, 220.0]), "max": np.array([100.0, 242.0])},
        ]
        bounds = [(item["min"], item["max"]) for item in geometry]
        self.assertEqual(baseline_order([0, 1, 2, 3], bounds, 22), [1, 0, 2, 3])
        for item in geometry:
            item["center"] = (item["min"] + item["max"]) / 2
        self.assertEqual(order_groups([[0], [1], [2], [3]], geometry), [[1], [0], [2], [3]])
        quads = [
            quad(134.4, 158.4, 72.2, 22.2),
            quad(27.4, 156.4, 87.2, 26.2),
            quad(219.3, 158.3, 82.4, 22.4),
            quad(25, 220, 75, 22),
        ]
        inputs, built_geometry = build_features(
            np.full((300, 400, 3), 255, np.uint8), quads
        )
        logits = np.full((len(inputs["edge_index"]),), 10, np.float32)
        self.assertEqual(group_lines(inputs, logits, built_geometry), [[1, 0, 2, 3]])

    def test_repeated_table_rows_override_column_gutters_without_changing_two_columns(self):
        table_quads = [
            quad(column * 140, row * 36, 90, 22)
            for row in range(4)
            for column in range(4)
        ]
        geometry = [
            {"min": value.min(0), "max": value.max(0), "center": value.mean(0)}
            for value in table_quads
        ]
        bounds = [(item["min"], item["max"]) for item in geometry]
        self.assertTrue(table_like_rows(list(range(16)), bounds, 22))
        self.assertEqual(
            order_groups([[index] for index in reversed(range(16))], geometry),
            [[index] for index in range(16)],
        )

        columns = [quad(0, row * 36) for row in range(4)] + [
            quad(300, row * 36) for row in range(4)
        ]
        geometry = [
            {"min": value.min(0), "max": value.max(0), "center": value.mean(0)}
            for value in columns
        ]
        bounds = [(item["min"], item["max"]) for item in geometry]
        self.assertFalse(table_like_rows(list(range(8)), bounds, 22))
        self.assertEqual(
            order_groups([[index] for index in range(8)], geometry),
            [[0], [1], [2], [3], [4], [5], [6], [7]],
        )

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

    def test_english_spacing_fusion_never_changes_non_whitespace_characters(self):
        characters = set("AB012.¥#")
        self.assertTrue(should_use_english_spacing("AA11 B", "AA 11  B", characters))
        self.assertFalse(should_use_english_spacing("AA  11", "AA 11", characters))
        self.assertFalse(should_use_english_spacing("AA 11", "A A11 ", characters))
        self.assertFalse(should_use_english_spacing("¥1.20", "#1.20", characters))
        self.assertFalse(should_use_english_spacing("中文 A", "A", characters))

    def test_vertical_crop_normalizes_geometry_before_orientation(self):
        image = np.zeros((120, 40, 3), np.uint8)
        image[:60] = [10, 20, 30]
        image[60:] = [100, 110, 120]
        crop, geometric_rotation = crop_line(image, quad(5, 10, 20, 100), include_geometry=True)
        self.assertEqual(geometric_rotation, 90)
        self.assertEqual(crop.shape, (48, 240, 3))
        self.assertLess(float(crop[:, :20].mean()), float(crop[:, -20:].mean()))

    def test_orientation_applies_only_high_confidence_180_and_validates_output(self):
        class FakeModel:
            def __init__(self, output):
                self.output = np.asarray(output, np.float32)
                self.input = None

            def run(self, names, values):
                self.input = values["x"]
                return [self.output]

        crop = np.zeros((48, 96, 3), np.uint8)
        crop[:, :48] = [10, 20, 30]
        crop[:, 48:] = [100, 110, 120]
        model = FakeModel([[.05, .95]])
        result, orientation = orient_line(crop, model, float("inf"))
        self.assertEqual(model.input.shape, (1, 3, 80, 160))
        self.assertEqual(orientation["classifiedAngle"], 180)
        self.assertTrue(orientation["applied180"])
        np.testing.assert_array_equal(result, crop[::-1, ::-1])

        result, orientation = orient_line(crop, FakeModel([[.3, .7]]), float("inf"))
        self.assertFalse(orientation["applied180"])
        np.testing.assert_array_equal(result, crop)
        with self.assertRaisesRegex(ValueError, "orientation_output_schema"):
            orient_line(crop, FakeModel([[.5, .4]]), float("inf"))

    def test_research_model_profiles_are_fixed_and_keep_product_default_small(self):
        profile, expected, limits = model_contract({})
        self.assertEqual(profile, "small-small")
        self.assertNotIn("rec", limits)
        self.assertEqual(expected["rec"], "5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634")
        profile, expected, limits = model_contract({"researchModelProfile": "medium-rec"})
        self.assertEqual(profile, "medium-rec")
        self.assertEqual(expected["rec"], "9c09abf0957f7968c7586464b7397b84ad2387a0497a351af40e9acc71b673ba")
        self.assertEqual(limits["rec"], 80 * 1024 * 1024)
        with self.assertRaisesRegex(ValueError, "research_model_profile"):
            model_contract({"researchModelProfile": "custom"})

    def test_tile_end_alignment_and_png_dimension_budget(self):
        self.assertEqual(tile_origins(1000), [0, 40])
        self.assertEqual(tile_origins(960), [0])
        png = b"\x89PNG\r\n\x1a\n" + b"\x00\x00\x00\rIHDR" + (20000).to_bytes(4, "big") + (1).to_bytes(4, "big") + bytes(9)
        with self.assertRaisesRegex(ValueError, "image_budget"):
            decode_png(png)


if __name__ == "__main__":
    unittest.main()
