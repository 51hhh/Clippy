//! Clippy: 有界 INCR 发送；所有状态由剪贴板服务线程持有，系统写入不等待接收者。
use super::*;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, Window};

pub(super) const IDLE_TIMEOUT: Duration = Duration::from_secs(4);
pub(super) const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TRANSFERS: usize = 4;
const MAX_RETAINED_BYTES: usize = 256 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 1024 * 1024;
type Key = (Window, Atom);

struct Transfer {
	snapshot: Arc<Vec<ClipboardData>>,
	index: usize,
	target: Atom,
	offset: usize,
	final_sent: bool,
	for_handover: bool,
	idle_until: Instant,
	expires: Instant,
}

impl Transfer {
	fn expired(&self, now: Instant) -> bool {
		now >= self.idle_until || now >= self.expires
	}
}

#[derive(Default)]
pub(super) struct Sender {
	transfers: HashMap<Key, Transfer>,
	subscriptions: HashMap<Window, (EventMask, usize)>,
}

/// ChangeProperty 固定头为 24 字节；再为 BIG-REQUESTS 头和四字节对齐留空间。
pub(super) fn direct_payload_limit(maximum_request_bytes: usize) -> usize {
	maximum_request_bytes.saturating_sub(32) & !3
}

impl Sender {
	pub(super) fn is_empty(&self) -> bool {
		self.transfers.is_empty()
	}
	pub(super) fn property_in_use(&self, window: Window, property: Atom) -> bool {
		self.transfers.contains_key(&(window, property))
	}

	pub(super) fn has_handover(&self) -> bool {
		self.transfers.values().any(|transfer| transfer.for_handover)
	}

	fn retained_bytes_with(&self, snapshot: &Arc<Vec<ClipboardData>>) -> Option<usize> {
		let mut snapshots = vec![snapshot];
		for transfer in self.transfers.values() {
			if !snapshots.iter().any(|other| Arc::ptr_eq(other, &transfer.snapshot)) {
				snapshots.push(&transfer.snapshot);
			}
		}
		snapshots
			.into_iter()
			.flat_map(|data| data.iter())
			.try_fold(0usize, |sum, data| sum.checked_add(data.bytes.len()))
	}

	pub(super) fn begin(
		&mut self,
		inner: &Inner,
		event: SelectionRequestEvent,
		snapshot: Arc<Vec<ClipboardData>>,
		index: usize,
	) -> Result<()> {
		let key = (event.requestor, event.property);
		if self.transfers.len() >= MAX_TRANSFERS
			|| self.transfers.contains_key(&key)
			|| !self.retained_bytes_with(&snapshot).is_some_and(|bytes| bytes <= MAX_RETAINED_BYTES)
			|| snapshot[index].bytes.len() > u32::MAX as usize
		{
			return Err(Error::unknown(
				"INCR transfer budget exhausted or property already in use",
			));
		}
		let connection = &inner.server.conn;
		if let Some((_, count)) = self.subscriptions.get_mut(&event.requestor) {
			*count += 1;
		} else {
			let previous = connection
				.get_window_attributes(event.requestor)
				.map_err(into_unknown)?
				.reply()
				.map_err(into_unknown)?
				.your_event_mask;
			connection
				.change_window_attributes(
					event.requestor,
					&ChangeWindowAttributesAux::new().event_mask(
						previous | EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY,
					),
				)
				.map_err(into_unknown)?
				.check()
				.map_err(into_unknown)?;
			self.subscriptions.insert(event.requestor, (previous, 1));
		}
		let length = snapshot[index].bytes.len() as u32;
		let now = Instant::now();
		self.transfers.insert(
			key,
			Transfer {
				snapshot,
				index,
				target: event.target,
				offset: 0,
				final_sent: false,
				for_handover: inner.handover_state.lock().active(),
				idle_until: now + IDLE_TIMEOUT,
				expires: now + TOTAL_TIMEOUT,
			},
		);
		let result = connection
			.change_property32(
				PropMode::REPLACE,
				event.requestor,
				event.property,
				inner.atoms.INCR,
				&[length],
			)
			.map_err(into_unknown)
			.and_then(|cookie| cookie.check().map_err(into_unknown));
		if result.is_err() {
			self.remove(inner, key, true);
		}
		result
	}

	fn remove(&mut self, inner: &Inner, key: Key, abort: bool) {
		if self.transfers.remove(&key).is_none() {
			return;
		}
		if abort {
			// 接收端会自行超时；清理残留属性，不伪造成功终止块。
			let _ = inner.server.conn.delete_property(key.0, key.1);
		}
		if let Some((previous, count)) = self.subscriptions.get_mut(&key.0) {
			*count -= 1;
			if *count == 0 {
				let previous = *previous;
				self.subscriptions.remove(&key.0);
				let _ = inner.server.conn.change_window_attributes(
					key.0,
					&ChangeWindowAttributesAux::new().event_mask(previous),
				);
			}
		}
		let _ = inner.server.conn.flush();
	}

	/// 返回 (是否完成, 是否属于 handover)；只有最终零块被确认才算完成。
	pub(super) fn acknowledge(
		&mut self,
		inner: &Inner,
		event: PropertyNotifyEvent,
	) -> Option<(bool, bool)> {
		if event.state != Property::DELETE {
			return None;
		}
		let key = (event.window, event.atom);
		let transfer = self.transfers.get_mut(&key)?;
		let for_handover = transfer.for_handover;
		if transfer.final_sent {
			self.remove(inner, key, false);
			return Some((true, for_handover));
		}
		let bytes = &transfer.snapshot[transfer.index].bytes;
		let chunk =
			direct_payload_limit(inner.server.conn.maximum_request_bytes()).min(MAX_CHUNK_BYTES);
		let end = transfer.offset.saturating_add(chunk).min(bytes.len());
		let result = inner
			.server
			.conn
			.change_property8(
				PropMode::APPEND,
				key.0,
				key.1,
				transfer.target,
				&bytes[transfer.offset..end],
			)
			.map_err(into_unknown)
			.and_then(|cookie| cookie.check().map_err(into_unknown));
		if let Err(error) = result {
			warn!("INCR transfer failed: {error}");
			self.remove(inner, key, true);
			return Some((false, for_handover));
		}
		transfer.final_sent = transfer.offset == bytes.len();
		transfer.offset = end;
		transfer.idle_until = Instant::now() + IDLE_TIMEOUT;
		None
	}

	pub(super) fn expire(&mut self, inner: &Inner, now: Instant) -> bool {
		let expired: Vec<_> = self
			.transfers
			.iter()
			.filter_map(|(key, transfer)| transfer.expired(now).then_some(*key))
			.collect();
		let handover_failed = expired.iter().any(|key| self.transfers[key].for_handover);
		for key in &expired {
			self.remove(inner, *key, true);
		}
		handover_failed
	}

	pub(super) fn abort_all(&mut self, inner: &Inner) {
		let keys: Vec<_> = self.transfers.keys().copied().collect();
		for key in keys {
			self.remove(inner, key, true);
		}
	}

	pub(super) fn destroy_requestor(&mut self, inner: &Inner, window: Window) -> bool {
		let keys: Vec<_> = self.transfers.keys().filter(|key| key.0 == window).copied().collect();
		let handover_failed = keys.iter().any(|key| self.transfers[key].for_handover);
		for key in &keys {
			self.remove(inner, *key, false);
		}
		handover_failed
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn transfer(snapshot: Arc<Vec<ClipboardData>>) -> Transfer {
		let now = Instant::now();
		Transfer {
			snapshot,
			index: 0,
			target: 1,
			offset: 0,
			final_sent: false,
			for_handover: false,
			idle_until: now + IDLE_TIMEOUT,
			expires: now + TOTAL_TIMEOUT,
		}
	}

	#[test]
	fn payload_budget_counts_shared_snapshots_once_and_releases_old_payloads() {
		let first = Arc::new(vec![ClipboardData { bytes: vec![1; 12], format: 1 }]);
		let second = Arc::new(vec![ClipboardData { bytes: vec![2; 8], format: 1 }]);
		let old = Arc::downgrade(&first);
		let mut sender = Sender::default();
		sender.transfers.insert((1, 1), transfer(first.clone()));
		sender.transfers.insert((2, 1), transfer(first.clone()));
		assert_eq!(sender.retained_bytes_with(&first), Some(12));
		assert_eq!(sender.retained_bytes_with(&second), Some(20));
		// SelectionClear / 新 selection 释放当前数据后，已接受的老快照仍可读。
		drop(first);
		assert_eq!(old.upgrade().unwrap()[0].bytes, vec![1; 12]);
		sender.transfers.remove(&(1, 1));
		assert!(old.upgrade().is_some());
		sender.transfers.remove(&(2, 1));
		assert!(old.upgrade().is_none());
		assert_eq!(sender.retained_bytes_with(&second), Some(8));
	}

	#[test]
	fn property_payload_bound_leaves_room_for_headers_and_padding() {
		for maximum in [32, 4096, 262140, 16777212, usize::MAX] {
			let payload = direct_payload_limit(maximum);
			assert_eq!(payload % 4, 0);
			assert!(payload <= maximum.saturating_sub(32));
		}
	}
	#[test]
	fn progress_cannot_extend_the_absolute_transfer_deadline() {
		let snapshot = Arc::new(vec![ClipboardData { bytes: vec![1], format: 1 }]);
		let mut transfer = transfer(snapshot);
		assert!(!transfer.expired(transfer.idle_until - Duration::from_millis(1)));
		assert!(transfer.expired(transfer.idle_until));
		transfer.idle_until = transfer.expires + IDLE_TIMEOUT;
		assert!(!transfer.expired(transfer.expires - Duration::from_millis(1)));
		assert!(transfer.expired(transfer.expires));
	}

	#[test]
	#[ignore = "requires private Xvfb and CLIPPY_TEST_X11_ISOLATED=1"]
	fn private_x11_rejects_oversized_property_before_reading_or_deleting_it() {
		assert_eq!(std::env::var("CLIPPY_TEST_X11_ISOLATED").as_deref(), Ok("1"));
		let context = XContext::new().unwrap();
		let atom = context
			.conn
			.intern_atom(false, b"CLIPPY_BOUNDED_PROPERTY")
			.unwrap()
			.reply()
			.unwrap()
			.atom;
		context
			.conn
			.change_property8(PropMode::REPLACE, context.win_id, atom, atom, &[7; 64])
			.unwrap()
			.check()
			.unwrap();
		assert!(get_property_bounded(&context.conn, context.win_id, atom, atom, 16).is_err());
		let unchanged = context
			.conn
			.get_property(false, context.win_id, atom, atom, 0, 0)
			.unwrap()
			.reply()
			.unwrap();
		assert_eq!(unchanged.bytes_after, 64, "拒绝前不读取/删除未授权的大属性");
		assert_eq!(
			get_property_bounded(&context.conn, context.win_id, atom, atom, 64).unwrap().value,
			vec![7; 64]
		);
		context
			.conn
			.change_property32(PropMode::REPLACE, context.win_id, atom, atom, &[1, 2])
			.unwrap()
			.check()
			.unwrap();
		assert!(get_property_bounded(&context.conn, context.win_id, atom, atom, 4).is_err());
	}
}
