use super::*;

const CALLER: &str = "recording-overlay-refresh";

fn microphone(id: &str) -> NativeRecordingAudioDevice {
    NativeRecordingAudioDevice {
        kind: RecordingAudioDeviceKind::Microphone,
        native_id: id.to_string(),
        label: format!("Microphone {id}"),
        is_default: false,
    }
}

fn selection(view: &RecordingAudioDeviceCatalogView) -> RecordingAudioSelection {
    RecordingAudioSelection {
        mode: RecordingAudioMode::Microphone,
        catalog_id: Some(view.catalog_id.clone()),
        system_device_id: None,
        microphone_device_id: Some(view.microphone_devices[0].id.clone()),
    }
}

// 后端不复制查询凭据；这里只主动复制以覆盖重复调用与晚回写的拒绝合同。
fn duplicate(refresh: &RecordingAudioDeviceRefresh) -> RecordingAudioDeviceRefresh {
    RecordingAudioDeviceRefresh {
        caller: refresh.caller.clone(),
        generation: refresh.generation,
    }
}

#[test]
fn audio_catalog_order_late_initial_success_keeps_retry_selection() {
    let registry = RecordingAudioDeviceCatalog::new();
    let initial = registry.begin_refresh(CALLER).unwrap();
    let retry = registry.begin_refresh(CALLER).unwrap();
    let retry_view = registry
        .complete_refresh(retry, vec![microphone("retry")])
        .unwrap();
    assert_eq!(
        registry.complete_refresh(initial, vec![microphone("initial")]),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert_eq!(
        registry.resolve(CALLER, &selection(&retry_view)),
        Ok(ResolvedRecordingAudioDevices {
            system_native_id: None,
            microphone_native_id: Some("retry".into()),
        })
    );
    assert_eq!(
        registry.resolve(CALLER, &selection(&retry_view)),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
}

#[test]
fn audio_catalog_order_late_failed_enumeration_keeps_retry_selection() {
    let registry = RecordingAudioDeviceCatalog::new();
    let initial = registry.begin_refresh(CALLER).unwrap();
    let retry = registry.begin_refresh(CALLER).unwrap();
    let view = registry
        .complete_refresh(retry, vec![microphone("retry")])
        .unwrap();
    // 宿主枚举失败仍发布空目录；旧失败结果不能清空新查询已发布的设备。
    assert_eq!(
        registry.complete_refresh(initial, Vec::new()),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert_eq!(
        registry
            .resolve(CALLER, &selection(&view))
            .unwrap()
            .microphone_native_id,
        Some("retry".into())
    );
}

#[test]
fn audio_catalog_order_pending_query_cannot_be_consumed_or_keep_old_tokens() {
    let registry = RecordingAudioDeviceCatalog::new();
    let previous = registry.refresh(CALLER, vec![microphone("old")]).unwrap();
    let pending = registry.begin_refresh(CALLER).unwrap();
    assert_eq!(
        registry.resolve(CALLER, &selection(&previous)),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    let pending_selection = RecordingAudioSelection {
        catalog_id: Some(pending.catalog_id()),
        ..RecordingAudioSelection::default_for(RecordingAudioMode::Microphone)
    };
    assert_eq!(
        registry.resolve(CALLER, &pending_selection),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    // 无 token 的系统默认选择不依赖查询是否完成，也不消费待发布目录。
    assert_eq!(
        registry.resolve(
            CALLER,
            &RecordingAudioSelection::default_for(RecordingAudioMode::Microphone)
        ),
        Ok(ResolvedRecordingAudioDevices::default())
    );
    let current = registry
        .complete_refresh(pending, vec![microphone("new")])
        .unwrap();
    assert_eq!(
        registry
            .resolve(CALLER, &selection(&current))
            .unwrap()
            .microphone_native_id,
        Some("new".into())
    );
}

#[test]
fn audio_catalog_order_duplicate_publish_cannot_replace_devices() {
    let registry = RecordingAudioDeviceCatalog::new();
    let refresh = registry.begin_refresh(CALLER).unwrap();
    let replay = duplicate(&refresh);
    let view = registry
        .complete_refresh(refresh, vec![microphone("current")])
        .unwrap();
    assert_eq!(
        registry.complete_refresh(replay, vec![microphone("replacement")]),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert_eq!(
        registry
            .resolve(CALLER, &selection(&view))
            .unwrap()
            .microphone_native_id,
        Some("current".into())
    );
}

#[test]
fn audio_catalog_order_consumed_catalog_cannot_be_revived() {
    let registry = RecordingAudioDeviceCatalog::new();
    let refresh = registry.begin_refresh(CALLER).unwrap();
    let replay = duplicate(&refresh);
    let view = registry.complete_refresh(refresh, Vec::new()).unwrap();
    assert_eq!(
        registry.resolve(
            CALLER,
            &RecordingAudioSelection {
                catalog_id: Some(view.catalog_id.clone()),
                ..RecordingAudioSelection::default_for(RecordingAudioMode::SystemAudio)
            }
        ),
        Ok(ResolvedRecordingAudioDevices::default())
    );
    assert_eq!(
        registry.complete_refresh(replay, vec![microphone("late")]),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert!(registry.by_caller.lock().unwrap().is_empty());
}

#[test]
fn audio_catalog_order_expired_pending_and_published_catalogs_stay_expired() {
    let registry = RecordingAudioDeviceCatalog::new();
    let expire = || {
        registry
            .by_caller
            .lock()
            .unwrap()
            .get_mut(CALLER)
            .unwrap()
            .created_at = Instant::now()
            .checked_sub(CATALOG_TTL + Duration::from_secs(1))
            .unwrap();
    };
    let expired = registry.begin_refresh(CALLER).unwrap();
    expire();
    assert_eq!(
        registry.complete_refresh(expired, vec![microphone("expired")]),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert!(registry.by_caller.lock().unwrap().is_empty());
    let refresh = registry.begin_refresh(CALLER).unwrap();
    let view = registry
        .complete_refresh(refresh, vec![microphone("current")])
        .unwrap();
    expire();
    assert_eq!(
        registry.resolve(CALLER, &selection(&view)),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert!(registry.by_caller.lock().unwrap().is_empty());
}

#[test]
fn audio_catalog_order_abandoned_pending_queries_share_the_global_bound() {
    let registry = RecordingAudioDeviceCatalog::new();
    let first = registry.begin_refresh("recording-overlay-oldest").unwrap();
    for index in 0..MAX_LIVE_CATALOGS {
        let refresh = registry
            .begin_refresh(&format!("recording-overlay-{index}"))
            .unwrap();
        if index % 2 == 0 {
            registry
                .complete_refresh(refresh, vec![microphone("bounded")])
                .unwrap();
        }
    }
    assert_eq!(registry.by_caller.lock().unwrap().len(), MAX_LIVE_CATALOGS);
    assert_eq!(
        registry.complete_refresh(first, vec![microphone("late")]),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert_eq!(registry.by_caller.lock().unwrap().len(), MAX_LIVE_CATALOGS);
}

#[test]
fn audio_catalog_order_other_callers_remain_independent_and_private() {
    let registry = RecordingAudioDeviceCatalog::new();
    let first = registry.begin_refresh(CALLER).unwrap();
    let second_caller = "recording-overlay-other";
    let second = registry.begin_refresh(second_caller).unwrap();
    let mut first_device = microphone("native-secret-a");
    first_device.label = "Microphone A".into();
    let mut second_device = microphone("native-secret-b");
    second_device.label = "Microphone B".into();
    let second_view = registry
        .complete_refresh(second, vec![second_device])
        .unwrap();
    let first_view = registry
        .complete_refresh(first, vec![first_device])
        .unwrap();
    for view in [&first_view, &second_view] {
        assert!(!serde_json::to_string(view)
            .unwrap()
            .contains("native-secret"));
    }
    assert_eq!(
        registry.resolve(second_caller, &selection(&first_view)),
        Err(RecordingAudioDeviceCatalogError::StaleCatalog)
    );
    assert_eq!(
        registry
            .resolve(CALLER, &selection(&first_view))
            .unwrap()
            .microphone_native_id,
        Some("native-secret-a".into())
    );
    assert_eq!(
        registry
            .resolve(second_caller, &selection(&second_view))
            .unwrap()
            .microphone_native_id,
        Some("native-secret-b".into())
    );
}
