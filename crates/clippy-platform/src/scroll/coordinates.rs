use anyhow::Result;

fn midpoint(size: (u32, u32), bounds: [f32; 4]) -> Result<[f64; 2]> {
    let [left, top, right, bottom] = crate::capture::pixel_rect(size.0, size.1, bounds)?;
    Ok([
        f64::midpoint(f64::from(left), f64::from(right)),
        f64::midpoint(f64::from(top), f64::from(bottom)),
    ])
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn physical(origin: [i32; 2], size: (u32, u32), bounds: [f32; 4]) -> Result<(i32, i32)> {
    let [x, y] = midpoint(size, bounds)?;
    Ok((
        i32::try_from(i64::from(origin[0]) + x.floor() as i64)?,
        i32::try_from(i64::from(origin[1]) + y.floor() as i64)?,
    ))
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn logical(
    origin: [f64; 2],
    points: [f64; 2],
    size: (u32, u32),
    bounds: [f32; 4],
) -> Result<(f64, f64)> {
    anyhow::ensure!(
        points.iter().all(|value| value.is_finite() && *value > 0.0)
            && origin.iter().all(|value| value.is_finite()),
        "Invalid logical display geometry"
    );
    let [x, y] = midpoint(size, bounds)?;
    Ok((
        origin[0] + x * points[0] / f64::from(size.0),
        origin[1] + y * points[1] / f64::from(size.1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_anchor_respects_negative_origin_and_source_pixels() {
        let point = physical([-2560, -320], (2560, 1440), [0.25, 0.125, 0.75, 0.625]).unwrap();
        assert_eq!(point, (-1280, 220));
    }

    #[test]
    fn retina_anchor_scales_the_crop_but_keeps_the_global_origin() {
        let point = logical(
            [-1728.0, -180.0],
            [1728.0, 1117.0],
            (3456, 2234),
            [0.25, 0.0, 0.75, 0.5],
        )
        .unwrap();
        assert_eq!(point, (-864.0, 99.25));
    }
}
