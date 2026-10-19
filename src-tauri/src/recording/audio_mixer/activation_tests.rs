struct ActivationInput {
    label: &'static str,
    events: Arc<Mutex<Vec<&'static str>>>,
    failed: bool,
    rollback_failed: bool,
}

impl RecordingAudioSource for ActivationInput {
    type Error = FixtureError;

    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> {
        self.events.lock().unwrap().push(self.label);
        if self.failed {
            Err(FixtureError("original activation error"))
        } else {
            Ok(Some(1_000_000_000))
        }
    }

    fn capture_next_available(
        &mut self,
        _: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        Ok(None)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(1_000_000_000)
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.events.lock().unwrap().push("rollback stop");
        if self.rollback_failed {
            Err(FixtureError("original rollback error"))
        } else {
            Ok(1_000_000_000)
        }
    }
}

type ActivationInputs = (
    MixedAudioSource<ActivationInput, ActivationInput>,
    Arc<Mutex<Vec<&'static str>>>,
);

fn activation_inputs(
    system_failed: bool,
    microphone_failed: bool,
    rollback_failed: bool,
) -> ActivationInputs {
    let events = Arc::new(Mutex::new(Vec::new()));
    (
        MixedAudioSource::new(
            ActivationInput {
                label: "system start",
                events: Arc::clone(&events),
                failed: system_failed,
                rollback_failed,
            },
            ActivationInput {
                label: "microphone start",
                events: Arc::clone(&events),
                failed: microphone_failed,
                rollback_failed: false,
            },
        ),
        events,
    )
}

#[test]
fn mixed_activation_starts_both_sources_in_order() {
    let (mut source, events) = activation_inputs(false, false, false);
    source.start_capture().unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        ["system start", "microphone start"]
    );
}

#[test]
fn first_activation_failure_does_not_start_second_source() {
    let (mut source, events) = activation_inputs(true, false, false);
    assert!(
        matches!(source.start_capture(), Err(MixedAudioSourceError::System(ref error)) if error == "original activation error")
    );
    assert_eq!(*events.lock().unwrap(), ["system start"]);
}

#[test]
fn second_activation_failure_stops_first_and_preserves_error() {
    let (mut source, events) = activation_inputs(false, true, false);
    assert!(
        matches!(source.start_capture(), Err(MixedAudioSourceError::Microphone(ref error)) if error == "original activation error")
    );
    assert_eq!(
        *events.lock().unwrap(),
        ["system start", "microphone start", "rollback stop"]
    );
}

#[test]
fn failed_activation_rollback_reports_both_original_errors() {
    let (mut source, events) = activation_inputs(false, true, true);
    assert!(
        matches!(source.start_capture(), Err(MixedAudioSourceError::ControlDiverged(ref error)) if error.contains("original activation error") && error.contains("original rollback error"))
    );
    assert_eq!(
        *events.lock().unwrap(),
        ["system start", "microphone start", "rollback stop"]
    );
}

struct LateInput {
    next: bool,
}

impl RecordingAudioSource for LateInput {
    type Error = FixtureError;

    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> {
        Ok(Some(2_000_000_000))
    }

    fn capture_next_available(
        &mut self,
        _: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if self.next {
            self.next = false;
            Ok(Some(constant(0, 96_000, 960, 2, 0.2)))
        } else {
            Ok(None)
        }
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(2_120_000_000)
    }
}

#[test]
fn delayed_mixed_activation_does_not_emit_historical_silence() {
    let mut source = MixedAudioSource::new(LateInput { next: true }, LateInput { next: true });
    assert_eq!(source.start_capture().unwrap(), Some(2_000_000_000));
    let output = source
        .capture_next_available(Duration::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(output.captured_at_ns, 2_000_000_000);
    assert_eq!(output.frame_count, 960);
    assert!(output
        .samples
        .iter()
        .all(|sample| (*sample - 0.2).abs() < f32::EPSILON));
}
