import unittest

from visual_paragraphs import merge_visual_paragraphs


def geometry(x, y, width, height):
    return {
        "min": [x, y],
        "max": [x + width, y + height],
        "center": [x + width / 2, y + height / 2],
    }


class VisualParagraphTests(unittest.TestCase):
    def test_merges_normal_rows_but_keeps_paragraph_gap_and_column_transition(self):
        items = [
            geometry(0, 0, 240, 28),
            geometry(0, 44, 260, 28),
            geometry(0, 88, 230, 28),
            geometry(0, 280, 250, 28),
            geometry(0, 324, 260, 28),
            geometry(500, 10, 250, 28),
            geometry(500, 54, 270, 28),
        ]
        self.assertEqual(
            merge_visual_paragraphs([[0], [1], [2], [3], [4], [5], [6]], items),
            [[0, 1, 2], [3, 4], [5, 6]],
        )

    def test_keeps_heading_separate_from_different_text_scale(self):
        items = [geometry(0, 0, 500, 56), geometry(0, 62, 280, 24)]
        self.assertEqual(merge_visual_paragraphs([[0], [1]], items), [[0], [1]])

    def test_preserves_existing_model_group_and_does_not_merge_side_by_side_rows(self):
        items = [
            geometry(0, 0, 200, 28),
            geometry(0, 44, 210, 28),
            geometry(260, 44, 210, 28),
        ]
        self.assertEqual(merge_visual_paragraphs([[0, 1], [2]], items), [[0, 1], [2]])


if __name__ == "__main__":
    unittest.main()
