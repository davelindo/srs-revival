//! One alternating-bit Sony connection carries standby queries and explicit sound controls.
use super::tandem::{self, Power};
use crate::power::Desired;
use crate::sound::{Action, Preset, Tuning};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default, PartialEq)]
enum Reconnect {
    #[default]
    Available,
    Pending,
    Attempted,
}

#[derive(Default)]
pub struct Link {
    sequence: u8,
    received: Option<u8>,
    ready: bool,
    connecting: bool,
    reconnect: Reconnect,
    pub observed: Power,
    desired: Desired,
    applied: bool,
    pending: VecDeque<Vec<u8>>,
    in_flight: bool,
    volume_max: Option<u8>,
    mute_control: Option<u8>,
    volume: Option<u8>,
    muted: Option<bool>,
    tuning: Tuning,
}

impl Link {
    fn queue(&mut self, payload: &[u8]) {
        assert!(self.pending.len() < 16, "native control backlog exceeded 16 commands");
        self.pending.push_back(payload.to_vec());
    }
    fn next(&mut self) -> Vec<Vec<u8>> {
        if self.in_flight {
            return Vec::new();
        }
        let Some(payload) = self.pending.pop_front() else {
            if self.connecting && self.ready {
                self.connecting = false;
                self.reconnect = Reconnect::Available;
            }
            return Vec::new();
        };
        self.in_flight = true;
        vec![tandem::envelope(0, self.sequence, &payload).expect("bounded native power command")]
    }
    pub fn begin(&mut self, desired: Desired) -> Vec<Vec<u8>> {
        self.ready = false;
        self.connecting = true;
        self.received = None;
        self.applied = false;
        self.observed = Power::default();
        self.desired = desired;
        self.sequence = 0;
        self.in_flight = false;
        self.pending.clear();
        self.volume_max = None;
        self.mute_control = None;
        self.volume = None;
        self.muted = None;
        self.tuning = Tuning::default();
        self.queue(&[4]);
        let mut messages = vec![vec![1, 0x22, 1, 0]];
        messages.extend(self.next());
        messages
    }
    pub fn apply(&mut self, desired: Desired) -> Vec<Vec<u8>> {
        self.reconnect = Reconnect::Available;
        self.begin(desired)
    }
    pub fn channel_closed(&mut self) {
        self.ready = false;
        if self.reconnect == Reconnect::Available {
            self.reconnect = Reconnect::Pending;
        }
    }
    pub fn reconnect_pending(&self) -> bool {
        self.reconnect == Reconnect::Pending
    }
    pub fn can_retry(&self) -> bool {
        self.reconnect_pending() || (self.connecting && self.reconnect == Reconnect::Available)
    }
    pub fn retry_after_timeout(&mut self) -> Option<Vec<Vec<u8>>> {
        // A warm restart can lose the initial handshake without an explicit
        // close. Spend the retry only on connection work, never on a sound
        // command whose write may already have reached the controller.
        if !self.can_retry() {
            return None;
        }
        self.reconnect = Reconnect::Attempted;
        Some(self.begin(self.desired))
    }
    fn queries(&mut self) {
        for category in [0x0fff, 0x1fff, 0x2fff] {
            self.queue(&tandem::query(category));
        }
    }
    pub fn sound(&mut self, action: Action) -> Result<Vec<Vec<u8>>, &'static str> {
        if !self.ready || self.in_flight || !self.pending.is_empty() {
            return Err("controller busy");
        }
        if action.tuning() {
            if matches!(action, Action::Bands(_)) {
                self.queue(&Action::Eq(Preset::Custom).tuning_write().unwrap());
            }
            if let Some(payload) = action.tuning_write() {
                self.queue(&payload);
            }
            self.tuning = Tuning::default();
            self.queue(&[0x91, 0x10, 0x0f, 0xff, 0]);
            self.queue(&[0x91, 0x11, 0xff, 0xff, 0]);
            self.queue(&[0x91, 0x20, 1, 0xff, 0xff, 0]);
            self.queue(&[0x91, 0x10, 0x1f, 0xff, 0]);
            return Ok(self.next());
        }
        match action {
            Action::Read => {}
            Action::Up | Action::Down => self.queue(&[0x93, 1, u8::from(action == Action::Up), 1]),
            Action::Mute(wanted) => {
                let current = self.muted.ok_or("read mute first")?;
                if current != wanted {
                    self.queue(&[0x93, 3, self.mute_control.ok_or("mute control unknown")?, u8::from(wanted)]);
                }
            }
            _ => return Err("unknown sound command"),
        }
        self.volume = None;
        self.muted = None;
        self.queue(&[0x91, 1]);
        self.queue(&[0x91, 3]);
        Ok(self.next())
    }
    pub fn sound_readback(&self) -> Option<String> {
        let maximum = self.volume_max.map(|n| n.to_string()).unwrap_or_else(|| "unknown".into());
        Some(format!("volume={}/{} mute={}", self.volume?, maximum, if self.muted? { "on" } else { "off" }))
    }
    pub fn sound_result(&self, action: Action) -> Option<Result<String, &'static str>> {
        if self.in_flight || !self.pending.is_empty() {
            return None;
        }
        if action.tuning() {
            return self.tuning.report(action);
        }
        if let Action::Mute(wanted) = action
            && self.muted? != wanted
        {
            return Some(Err("Sony reported a different mute setting"));
        }
        self.sound_readback().map(Ok)
    }
    pub fn complete(&self) -> bool {
        !self.in_flight
            && self.pending.is_empty()
            && self.observed.network_standby == Some(self.desired.network)
            && self
                .observed
                .auto_standby
                .is_some_and(|value| self.desired.automatic.is_none_or(|wanted| value == wanted))
            && self.observed.battery.is_some()
    }
    pub fn incoming(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let Some(frame) = tandem::receive(bytes) else {
            return Vec::new();
        };
        if frame.sequence > 1 {
            return Vec::new();
        }
        if frame.kind == 1 {
            // Sony ACK carries the next alternating bit. Keep one command in
            // flight; the retained capture starts with data 0 -> ACK 1.
            if self.in_flight && frame.sequence == 1 - self.sequence {
                self.sequence = frame.sequence;
                self.in_flight = false;
            }
            return self.next();
        }
        if frame.kind != 0 {
            return Vec::new();
        }
        let mut replies = vec![tandem::envelope(1, 1 - frame.sequence, &[]).unwrap()];
        if self.received == Some(frame.sequence) {
            return replies;
        }
        self.received = Some(frame.sequence);
        self.tuning.incoming(&frame.payload);
        match frame.payload.as_slice() {
            [6, 1, _, maximum, ..] => self.volume_max = Some(*maximum),
            [6, 3, control @ (0 | 1), ..] => self.mute_control = Some(*control),
            [0x92, 1, value] => self.volume = Some(*value),
            [0x92, 3, value @ (0 | 1)] => self.muted = Some(*value == 1),
            _ => {}
        }
        match frame.payload.as_slice() {
            [0, ..] => {
                self.pending.clear();
                self.ready = false;
                self.connecting = true;
                self.queue(&[0x10, 2, 1]);
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                let bcd = |value: u64| ((value / 10) * 16 + value % 10) as u8;
                self.queue(&[1, bcd(now / 3600 % 24), bcd(now / 60 % 60), bcd(now % 60)]);
                self.queue(&[5, 0, 0, 1]);
            }
            // The physical controller ends its capability list with ConnectLinkComplete (03).
            // A Sony network status can arrive later on the stock player.
            [3] | [0xf0, 1, 1, 0] if !self.ready => {
                self.ready = true;
                eprintln!("Sony native controller ready for status queries");
                self.queries();
            }
            payload if self.ready => {
                self.observed.incoming(payload);
                if !self.applied && self.observed.network_standby.is_some() && self.observed.auto_standby.is_some() {
                    self.applied = true;
                    if self.observed.network_standby != Some(self.desired.network) {
                        self.queue(&tandem::set(0x0fff, self.desired.network));
                        self.queue(&tandem::query(0x0fff));
                    }
                    if let Some(wanted) = self.desired.automatic
                        && self.observed.auto_standby != Some(wanted)
                    {
                        self.queue(&tandem::set(0x1fff, wanted));
                        self.queue(&tandem::query(0x1fff));
                    }
                }
            }
            _ => {}
        }
        replies.extend(self.next());
        replies
    }
    pub fn report(&self, status: &str) -> String {
        let boolean = |value: Option<bool>| value.map_or("unknown", |value| if value { "on" } else { "off" });
        let battery = match self.observed.battery {
            Some(0x20ff) => "Charging",
            Some(0x21ff) => "About 100%",
            Some(0x22ff) => "About 80%",
            Some(0x23ff) => "About 60%",
            Some(0x24ff) => "About 40%",
            Some(0x25ff) => "About 20%",
            Some(0x26ff) => "About 10%",
            Some(0x27ff) => "About 0%",
            _ => "Unknown",
        };
        format!(
            "status={status}\nnetwork_standby={}\nauto_standby={}\nbattery={battery}\n",
            boolean(self.observed.network_standby),
            boolean(self.observed.auto_standby)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mc(sequence: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = tandem::envelope(0, sequence, payload).unwrap();
        frame[1] = 0x21;
        frame
    }
    fn payloads(frames: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        frames
            .into_iter()
            .filter_map(|mut frame| {
                frame[1] = 0x21;
                let frame = tandem::receive(&frame)?;
                (frame.kind == 0).then_some(frame.payload)
            })
            .collect()
    }
    fn ack(bit: u8) -> Vec<u8> {
        // Retained live capture: ACK of data 0 is type 1, bit 1,
        // no payload, checksum 2. ACK 0 has checksum 1.
        vec![1, 0x21, 9, 0, 0, 0x3e, 1, bit, 0, 0, 0, 0, 1 + bit, 0x3c]
    }
    #[test]
    fn closing_channel_zero_during_startup_allows_one_fresh_handshake() {
        let mut link = Link::default();
        let opening = link.begin(Desired { network: true, automatic: Some(false) });
        assert_eq!(opening[0], [1, 0x22, 1, 0]);
        assert_eq!(payloads(opening.clone()), [vec![4]]);
        link.incoming(&ack(1));
        link.incoming(&mc(0, &[0, 0x30, 0, 0x31, 1, 5]));

        link.channel_closed();
        assert!(link.reconnect_pending());
        assert_eq!(link.retry_after_timeout(), Some(opening));
        link.channel_closed();
        assert!(!link.reconnect_pending());

        link.incoming(&ack(1));
        assert_eq!(payloads(link.incoming(&mc(0, &[3]))), [tandem::query(0x0fff).to_vec()]);
        assert!(link.ready);
        assert_eq!(link.desired.automatic, Some(false));
        for bit in [0, 1, 0] {
            link.incoming(&ack(bit));
        }
        link.channel_closed();
        assert!(link.reconnect_pending());
    }

    #[test]
    fn a_stalled_startup_renegotiates_once_without_a_close_or_sound_replay() {
        let mut link = Link::default();
        let opening = link.begin(Desired::default());
        assert!(!link.reconnect_pending());
        assert!(link.sound(Action::Read).is_err());
        assert_eq!(link.retry_after_timeout(), Some(opening));

        link.incoming(&ack(1));
        assert_eq!(payloads(link.incoming(&mc(0, &[3]))), [tandem::query(0x0fff).to_vec()]);
        assert_eq!(link.retry_after_timeout(), None);
        for bit in [0, 1, 0] {
            link.incoming(&ack(bit));
        }
        assert_eq!(payloads(link.sound(Action::Up).unwrap()), [vec![0x93, 1, 1, 1]]);
        assert_eq!(link.retry_after_timeout(), None);
    }

    #[test]
    fn a_master_volume_step_waits_for_both_sony_readbacks() {
        let mut link = Link { ready: true, ..Link::default() };
        link.incoming(&mc(0, &[6, 1, 0, 31]));
        link.incoming(&mc(1, &[6, 3, 0]));
        link.incoming(&mc(0, &[0x92, 1, 9]));
        link.incoming(&mc(1, &[0x92, 3, 0]));
        assert_eq!(link.sound_readback().as_deref(), Some("volume=9/31 mute=off"));
        assert_eq!(payloads(link.sound(Action::Up).unwrap()), vec![vec![0x93, 1, 1, 1]]);
        assert_eq!(link.sound_readback(), None);
        assert!(link.sound(Action::Up).is_err());
        assert_eq!(payloads(link.incoming(&ack(1))), vec![vec![0x91, 1]]);
        assert_eq!(payloads(link.incoming(&ack(0))), vec![vec![0x91, 3]]);
        link.incoming(&ack(1));
        link.incoming(&mc(0, &[0x92, 1, 10]));
        assert_eq!(link.sound_readback(), None);
        link.incoming(&mc(1, &[0x92, 3, 0]));
        assert_eq!(link.sound_readback().as_deref(), Some("volume=10/31 mute=off"));
    }

    #[test]
    fn custom_eq_uses_sony_steps_and_waits_for_all_bands() {
        let mut link = Link { ready: true, ..Link::default() };
        let action = Action::Bands([-10, -1, 0, 1, 10]);
        assert_eq!(payloads(link.sound(action).unwrap()), [vec![0x93, 0x10, 5, 0xff, 0, 0]]);
        assert_eq!(
            payloads(link.incoming(&ack(1))),
            [vec![0x93, 0x10, 0x10, 0xff, 0x10, 10, 1, 1, 2, 10, 3, 11, 4, 12, 5, 21]]
        );
        for (bit, query) in [
            (0, vec![0x91, 0x10, 0x0f, 0xff, 0]),
            (1, vec![0x91, 0x11, 0xff, 0xff, 0]),
            (0, vec![0x91, 0x20, 1, 0xff, 0xff, 0]),
            (1, vec![0x91, 0x10, 0x1f, 0xff, 0]),
        ] {
            assert_eq!(payloads(link.incoming(&ack(bit))), [query]);
        }
        link.incoming(&ack(0));
        link.incoming(&mc(0, &[0x92, 0x10, 5, 0xff, 0, 0, 0, 0, 0]));
        link.incoming(&mc(1, &[0x92, 0x11, 0xff, 0xff, 0, 0, 0, 1, 1, 0]));
        link.incoming(&mc(0, &[0x92, 0x20, 1, 0x0f, 0xff, 0, 0, 0, 0, 0]));
        assert_eq!(link.sound_result(action), None);
        link.incoming(&mc(1, &[0x92, 0x10, 0x10, 0xff, 0, 0, 0, 0x10, 10, 1, 1, 2, 10, 3, 11, 4, 12, 5, 21]));
        assert_eq!(link.sound_result(action), Some(Ok("eq=5 clear=0 wide=1 bands=-10,-1,0,1,10".into())));
        assert_eq!(link.sound_result(Action::Wide(false)), Some(Err("Sony reported a different sound setting")));
    }
    #[test]
    fn physical_capability_terminator_starts_queries_once() {
        let mut link = Link::default();
        assert_eq!(payloads(link.incoming(&mc(0, &[3]))), vec![tandem::query(0x0fff).to_vec()]);
        assert!(link.ready);
        assert!(payloads(link.incoming(&mc(0, &[3]))).is_empty());
        assert!(payloads(link.incoming(&mc(1, &[0xf0, 1, 1, 0]))).is_empty());
        assert_eq!(payloads(link.incoming(&ack(1))), vec![tandem::query(0x1fff).to_vec()]);
    }
    #[test]
    fn native_settings_wait_for_handshake_and_observed_state_then_require_readback() {
        let mut link = Link::default();
        assert_eq!(payloads(link.begin(Desired::default())), vec![vec![4]]);
        assert!(!link.ready);
        assert!(link.incoming(&ack(1)).is_empty());
        let connected = mc(0, &[0, 0x30, 0, 0x31, 1, 5]);
        assert_eq!(payloads(link.incoming(&connected)), vec![vec![0x10, 2, 1]]);
        assert!(payloads(link.incoming(&connected)).is_empty());
        let clock = payloads(link.incoming(&ack(0)));
        assert_eq!(clock[0][0], 1);
        assert_eq!(payloads(link.incoming(&ack(1))), vec![vec![5, 0, 0, 1]]);
        assert!(link.incoming(&ack(0)).is_empty());
        assert_eq!(payloads(link.incoming(&mc(1, &[0xf0, 1, 1, 0]))), vec![tandem::query(0x0fff).to_vec()]);
        assert!(link.ready);
        assert_eq!(payloads(link.incoming(&ack(1))), vec![tandem::query(0x1fff).to_vec()]);
        assert_eq!(payloads(link.incoming(&ack(0))), vec![tandem::query(0x2fff).to_vec()]);
        assert!(link.incoming(&ack(1)).is_empty());
        assert!(payloads(link.incoming(&mc(0, &[0xf3, 0x12, 0x0f, 0xff, 0, 0, 1, 1, 0]))).is_empty());
        assert_eq!(
            payloads(link.incoming(&mc(1, &[0xf3, 0x12, 0x1f, 0xff, 0, 0, 1, 1, 1]))),
            vec![tandem::set(0x0fff, true).to_vec()]
        );
        assert_eq!(payloads(link.incoming(&ack(0))), vec![tandem::query(0x0fff).to_vec()]);
        assert!(link.incoming(&ack(1)).is_empty());
        link.incoming(&mc(0, &[0xf3, 0x12, 0x21, 0xff, 0, 0, 0, 0]));
        assert!(!link.complete());
        link.incoming(&mc(1, &[0xf3, 0x12, 0x0f, 0xff, 0, 0, 1, 1, 1]));
        assert!(link.complete());
    }
}
