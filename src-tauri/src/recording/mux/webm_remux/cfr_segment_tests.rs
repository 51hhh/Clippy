use super::*;

fn fractional_segment(duration_ns: u64, frames: u64) -> WebmRemuxSource {
    WebmRemuxSource {
        path: PathBuf::from("unused.webm"),
        byte_length: 1,
        sha256: String::new(),
        started_at_ns: 66_666_666,
        duration_ns,
        frame_count: frames,
    }
}

#[test]
fn cfr_nanosecond_endpoint_quantization_accepts_one_real_frame() {
    let spec = WebmRemuxSpec {
        width: 2,
        height: 2,
        fps_numerator: 15,
        fps_denominator: 1,
    };
    assert!(validate_frame_timestamp(0, 0, spec, &fractional_segment(66_666_667, 1)).is_ok());
}

#[test]
fn cfr_nanosecond_endpoint_quantization_accepts_two_adjacent_global_slots() {
    let spec = WebmRemuxSpec {
        width: 2,
        height: 2,
        fps_numerator: 15,
        fps_denominator: 1,
    };
    let source = fractional_segment(133_333_334, 2);
    assert!(validate_frame_timestamp(0, 0, spec, &source).is_ok());
    assert!(validate_frame_timestamp(66_666_666, 1, spec, &source).is_ok());
}

#[test]
fn cfr_endpoint_quantization_rejects_non_cfr_global_origins() {
    let spec = WebmRemuxSpec {
        width: 2,
        height: 2,
        fps_numerator: 15,
        fps_denominator: 1,
    };
    for start in [0, 1, 66_666_665, 66_666_667, u64::MAX] {
        let mut source = fractional_segment(66_666_667, 1);
        source.started_at_ns = start;
        assert!(validate_frame_timestamp(0, 0, spec, &source).is_err());
    }
}

#[test]
fn cfr_endpoint_quantization_rejects_a_longer_interval() {
    let spec = WebmRemuxSpec {
        width: 2,
        height: 2,
        fps_numerator: 15,
        fps_denominator: 1,
    };
    assert!(validate_frame_timestamp(0, 0, spec, &fractional_segment(66_666_668, 1)).is_err());
    assert!(validate_frame_timestamp(0, 0, spec, &fractional_segment(100_000_000, 1)).is_err());
}

#[test]
fn cfr_endpoint_quantization_keeps_invalid_counts_and_timestamps_rejected() {
    let spec = WebmRemuxSpec {
        width: 2,
        height: 2,
        fps_numerator: 15,
        fps_denominator: 1,
    };
    assert!(validate_frame_timestamp(0, 0, spec, &fractional_segment(66_666_667, 0)).is_err());
    assert!(validate_frame_timestamp(0, 0, spec, &fractional_segment(66_666_666, 2)).is_err());
    assert!(
        validate_frame_timestamp(1_000_000, 0, spec, &fractional_segment(66_666_667, 1)).is_err()
    );
}
