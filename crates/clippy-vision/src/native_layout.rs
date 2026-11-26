use clippy_ocr_protocol::Line;

struct Bounds {
    min: [f64; 2],
    max: [f64; 2],
}

impl Bounds {
    fn from_line(line: &Line) -> Self {
        let mut min = line.quad[0];
        let mut max = min;
        for point in line.quad.into_iter().skip(1) {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        Self { min, max }
    }

    fn extent(&self, axis: usize) -> f64 {
        (self.max[axis] - self.min[axis]).max(1.0)
    }
}

pub(super) fn group_lines(lines: &[Line]) -> Vec<Vec<usize>> {
    let indices: Vec<_> = (0..lines.len()).collect();
    match lines.len() {
        0 => return Vec::new(),
        1 => return vec![indices],
        _ => {}
    }
    let boxes: Vec<_> = lines.iter().map(Bounds::from_line).collect();
    let line_height = median(boxes.iter().map(|bounds| bounds.extent(1)).collect());
    let layout = Layout { boxes, line_height };
    let mut paragraphs = Vec::new();
    layout.partition(indices, &mut paragraphs);
    paragraphs
}

struct Layout {
    boxes: Vec<Bounds>,
    line_height: f64,
}

impl Layout {
    fn partition(&self, indices: Vec<usize>, paragraphs: &mut Vec<Vec<usize>>) {
        if indices.len() == 1 {
            paragraphs.push(indices);
            return;
        }
        let mut rows = self.baseline_rows(&indices);
        let dense_rows: Vec<_> = rows.iter().filter(|row| row.len() >= 3).collect();
        if dense_rows.len() >= 3
            && dense_rows.iter().map(|row| row.len()).sum::<usize>() >= indices.len().div_ceil(2)
        {
            paragraphs.extend(rows);
            return;
        }

        // 离开正文足够远的首行可横跨各列，先读这一行再判断下方分栏。
        if rows.len() >= 3
            && self.min_start(&rows[1], 1) - self.max_end(&rows[0], 1) >= 2.0 * self.line_height
        {
            paragraphs.push(rows.remove(0));
            self.partition(rows.into_iter().flatten().collect(), paragraphs);
            return;
        }
        if let Some((before, after)) = self
            .split_at_gap(&indices, 0, 1.5 * self.line_height, |left, right| {
                self.separate_columns(left, right)
            })
            .or_else(|| self.split_at_gap(&indices, 1, self.line_height, |_, _| true))
        {
            self.partition(before, paragraphs);
            self.partition(after, paragraphs);
            return;
        }

        let first_paragraph = paragraphs.len();
        for row in rows {
            if row.len() == 1 && paragraphs.len() > first_paragraph {
                let previous = paragraphs.last_mut().expect("当前块已有段落");
                if self.same_paragraph(*previous.last().expect("段落含有行"), row[0]) {
                    previous.push(row[0]);
                    continue;
                }
            }
            paragraphs.push(row);
        }
    }

    fn baseline_rows(&self, indices: &[usize]) -> Vec<Vec<usize>> {
        let mut rows: Vec<Vec<usize>> = Vec::new();
        for index in self.sorted_on(indices, 1) {
            if let Some(row) = rows.last_mut() {
                let offset = self.boxes[index].min[1] - self.boxes[row[0]].min[1];
                if offset <= 0.5 * self.line_height {
                    row.push(index);
                    continue;
                }
            }
            rows.push(vec![index]);
        }
        for row in &mut rows {
            row.sort_by(|&left, &right| {
                self.boxes[left].min[0]
                    .total_cmp(&self.boxes[right].min[0])
                    .then_with(|| self.boxes[left].min[1].total_cmp(&self.boxes[right].min[1]))
                    .then_with(|| left.cmp(&right))
            });
        }
        rows
    }

    fn split_at_gap(
        &self,
        indices: &[usize],
        axis: usize,
        minimum: f64,
        accepts: impl Fn(&[usize], &[usize]) -> bool,
    ) -> Option<(Vec<usize>, Vec<usize>)> {
        let mut ordered = self.sorted_on(indices, axis);
        let mut end = self.boxes[ordered[0]].max[axis];
        let mut largest = minimum;
        let mut cut = None;
        for (at, &index) in ordered.iter().enumerate().skip(1) {
            let bounds = &self.boxes[index];
            let gap = bounds.min[axis] - end;
            if gap > largest && accepts(&ordered[..at], &ordered[at..]) {
                cut = Some(at);
                largest = gap;
            }
            end = end.max(bounds.max[axis]);
        }
        let after = ordered.split_off(cut?);
        Some((ordered, after))
    }

    fn separate_columns(&self, left: &[usize], right: &[usize]) -> bool {
        // 只有两边都有多行正文才切列，尾部状态、页码和窄数值留在原行。
        let left_rows = self.baseline_rows(left);
        let right_rows = self.baseline_rows(right);
        let shorter = left_rows.len().min(right_rows.len());
        let longer = left_rows.len().max(right_rows.len());
        if shorter < 2 || shorter * 5 < longer * 2 {
            return false;
        }
        let top = self.min_start(left, 1).max(self.min_start(right, 1));
        let bottom = self.max_end(left, 1).min(self.max_end(right, 1));
        bottom - top >= self.line_height
            && [left, right].into_iter().all(|column| {
                median(
                    column
                        .iter()
                        .map(|&index| self.boxes[index].extent(0))
                        .collect(),
                ) >= 2.0 * self.line_height
            })
    }

    fn same_paragraph(&self, above: usize, below: usize) -> bool {
        let (above, below) = (&self.boxes[above], &self.boxes[below]);
        let gap = below.min[1] - above.max[1];
        let overlap = above.max[0].min(below.max[0]) - above.min[0].max(below.min[0]);
        let overlap_ratio = overlap / above.extent(0).min(below.extent(0));
        let height_ratio =
            above.extent(1).max(below.extent(1)) / above.extent(1).min(below.extent(1));
        (-0.15 * self.line_height..=self.line_height).contains(&gap)
            && overlap_ratio >= 0.25
            && height_ratio <= 1.8
            && ((above.min[0] - below.min[0]).abs() <= 1.5 * self.line_height
                || overlap_ratio >= 0.6)
    }

    fn sorted_on(&self, indices: &[usize], axis: usize) -> Vec<usize> {
        let mut sorted = indices.to_vec();
        sorted.sort_by(|&a, &b| {
            self.boxes[a].min[axis]
                .total_cmp(&self.boxes[b].min[axis])
                .then_with(|| a.cmp(&b))
        });
        sorted
    }

    fn max_end(&self, indices: &[usize], axis: usize) -> f64 {
        indices
            .iter()
            .map(|&i| self.boxes[i].max[axis])
            .fold(f64::NEG_INFINITY, f64::max)
    }

    fn min_start(&self, indices: &[usize], axis: usize) -> f64 {
        indices
            .iter()
            .map(|&i| self.boxes[i].min[axis])
            .fold(f64::INFINITY, f64::min)
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    let middle = (values.len() - 1) / 2;
    *values.select_nth_unstable_by(middle, f64::total_cmp).1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x: f64, y: f64, width: f64, height: f64) -> Line {
        Line {
            text: "line".into(),
            quad: [
                [x, y],
                [x + width, y],
                [x + width, y + height],
                [x, y + height],
            ],
            confidence: 0.9,
        }
    }

    #[test]
    fn header_precedes_two_columns_and_each_column_keeps_its_paragraph() {
        let lines = [
            line(20.0, 12.0, 320.0, 24.0),
            line(950.0, 12.0, 100.0, 24.0),
            line(20.0, 98.0, 320.0, 24.0),
            line(620.0, 98.0, 310.0, 24.0),
            line(20.0, 134.0, 240.0, 24.0),
            line(625.0, 135.0, 240.0, 24.0),
        ];
        assert_eq!(group_lines(&lines), [vec![0, 1], vec![2, 4], vec![3, 5]]);
    }

    #[test]
    fn repeated_table_cells_remain_in_visual_rows() {
        let mut lines = Vec::new();
        for row in 0..3 {
            for column in 0..3 {
                lines.push(line(
                    260.0 * f64::from(column),
                    40.0 * f64::from(row),
                    90.0,
                    22.0,
                ));
            }
        }
        assert_eq!(
            group_lines(&lines),
            [vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]]
        );
    }

    #[test]
    fn larger_heading_and_large_paragraph_gap_are_separate() {
        let lines = [
            line(10.0, 0.0, 420.0, 50.0),
            line(10.0, 58.0, 240.0, 24.0),
            line(10.0, 94.0, 260.0, 24.0),
            line(10.0, 180.0, 220.0, 24.0),
        ];
        assert_eq!(group_lines(&lines), [vec![0], vec![1, 2], vec![3]]);
    }
}
