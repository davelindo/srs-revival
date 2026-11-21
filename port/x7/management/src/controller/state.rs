//! Sony's source-ownership and standby contract. See docs/sony-port.md for
//! original producer/consumer addresses. No B&W native registers are used.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Airplay,
    Spotify,
    Snapcast,
}

impl Source {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "airplay" => Some(Self::Airplay),
            "spotify" => Some(Self::Spotify),
            "snapcast" => Some(Self::Snapcast),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Airplay => "airplay",
            Self::Spotify => "spotify",
            Self::Snapcast => "snapcast",
        }
    }
    fn prefix(self) -> u8 {
        if self == Self::Spotify { 5 } else { 3 }
    }
    fn index(self) -> usize {
        match self {
            Self::Airplay => 0,
            Self::Spotify => 1,
            Self::Snapcast => 2,
        }
    }
    fn remote(self, command: u8, value: u8) -> Effect {
        Effect::Remote([self.prefix(), command, 1, value])
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Fixed(u8, [u8; 2]),
    Remote([u8; 4]),
    Broadcast(String),
    Restart(Source),
    Recovery,
}

#[derive(Default)]
pub struct State {
    pub owner: Option<Source>,
    pub granted: bool,
    requesting: bool,
    active: [bool; 3],
    pub sleeping: bool,
    pub volume: Option<u8>,
    pending_ack: Option<(u8, u8)>,
    teardown_owner: Option<Source>,
    restarting: Option<Source>,
    desired_volume: Option<u8>,
    next: Option<Source>,
}

fn status(register: u8, value: u8) -> Effect {
    Effect::Fixed(register, [value, 0])
}
fn text(line: &str) -> Effect {
    Effect::Broadcast(line.into())
}

impl State {
    pub fn request(&mut self, source: Source) -> Vec<Effect> {
        if self.restarting.is_some() {
            self.next = Some(source);
            return Vec::new();
        }
        if self.owner == Some(source) && self.granted {
            return Vec::new();
        }
        self.owner = Some(source);
        self.teardown_owner = None;
        self.granted = false;
        self.requesting = true;
        self.pending_ack = None;
        let mut effects = vec![text("audio off"), text("yield sony")];
        if self.sleeping {
            self.sleeping = false;
            effects.push(status(0xe1, 4));
        }
        effects.push(source.remote(1, 0));
        effects
    }

    fn park(&mut self) -> Vec<Effect> {
        self.granted = false;
        self.requesting = false;
        vec![text("audio off"), text("yield sony")]
    }

    fn stopped_ack(&mut self, register: u8, value: u8) -> Vec<Effect> {
        let mut effects = self.park();
        if self.playing() {
            self.pending_ack = Some((register, value));
        } else {
            effects.push(status(register, value));
        }
        effects
    }

    pub fn playback(&mut self, source: Source, playing: bool) -> Vec<Effect> {
        // Only an explicit intent may take ownership from another receiver.
        if playing && self.owner != Some(source) {
            return Vec::new();
        }
        let was = self.active[source.index()];
        self.active[source.index()] = playing;
        let mut effects = Vec::new();
        if !playing {
            if !self.playing()
                && let Some((register, value)) = self.pending_ack.take()
            {
                effects.push(status(register, value));
            } else if was && self.granted && self.owner == Some(source) {
                effects.push(source.remote(1, 6));
            }
        }
        effects
    }

    pub fn gone(&mut self, source: Source) -> Vec<Effect> {
        self.active[source.index()] = false;
        let was_owner = self.owner == Some(source);
        let mut effects = if was_owner { self.park() } else { Vec::new() };
        if was_owner {
            self.owner = None;
        }
        if self.restarting == Some(source) {
            self.restarting = None;
            effects.push(status(0xe3, 5));
        }
        if !self.playing()
            && let Some((register, value)) = self.pending_ack.take()
        {
            effects.push(status(register, value));
        }
        if self.restarting.is_none()
            && let Some(next) = self.next.take()
        {
            effects.extend(self.request(next));
        }
        effects
    }

    pub fn playing(&self) -> bool {
        self.active.iter().any(|playing| *playing)
    }

    pub fn volume(&mut self, value: u8) -> Vec<Effect> {
        let value = value.min(100);
        self.desired_volume = Some(value);
        self.volume = Some(value);
        let mut effects = vec![text(&format!("volume {value}"))];
        if self.granted
            && let Some(owner) = self.owner
        {
            effects.push(owner.remote(3, value));
        }
        effects
    }

    pub fn incoming(&mut self, register: u8, bytes: &[u8]) -> Vec<Effect> {
        match (register, bytes) {
            // KnOS 0x601bd678 reports no Sony-stored Spotify accounts.
            // The replacement's Spotify receiver manages its own session.
            (0xf9, [1, 0]) => vec![Effect::Fixed(0xf9, [0x21, 0x21])],
            (0x28, [0x21, _]) => vec![Effect::Fixed(0x29, [0x20, 0x20]), Effect::Recovery],
            (0xe1, [1, _]) => {
                self.sleeping = true;
                self.stopped_ack(0xe1, 3)
            }
            (0xe1, [2, _]) => {
                self.sleeping = false;
                vec![status(0xe1, 4)]
            }
            (0x48, [0xf6, _]) => vec![status(0xe3, 1)],
            (0x48, [0xf8, _]) => self.stopped_ack(0xe3, 4),
            (0x48, [0xf7, _]) => {
                self.teardown_owner = self.owner;
                self.stopped_ack(0xe3, 0)
            }
            (0x48, [0xf9, _]) => {
                // 0x5813fb14 uses F9 during activation. Only the teardown
                // worker at 0x5813fdf6 precedes its F9 with F7.
                if self.restarting.is_some() {
                    Vec::new()
                } else if let Some(owner) = self.teardown_owner.take().filter(|owner| Some(*owner) == self.owner) {
                    self.restarting = Some(owner);
                    vec![Effect::Restart(owner)]
                } else {
                    vec![status(0xe3, 5)]
                }
            }
            (0x40, [3, 0x0f, 1, 0x10]) => {
                self.teardown_owner = None;
                vec![Effect::Remote([3, 0x0f, 1, 0x11])]
            }
            (0x40, [prefix @ (3 | 5), 0x0f, 1, command @ (0 | 0x10)]) => {
                let mut effects = self.park();
                effects.push(Effect::Remote([*prefix, 0x0f, 1, command + 1]));
                effects
            }
            (0x40, [prefix @ (3 | 5), 3, 1, value]) => {
                if self.owner.is_some_and(|owner| owner.prefix() == *prefix) {
                    self.volume = Some((*value).min(100));
                    vec![text(&format!("volume {}", self.volume.unwrap()))]
                } else {
                    Vec::new()
                }
            }
            (0x40, [prefix @ (3 | 5), 1, 1, answer]) => {
                let Some(owner) = self.owner.filter(|owner| owner.prefix() == *prefix) else {
                    return Vec::new();
                };
                match answer {
                    2 if self.requesting => {
                        vec![owner.remote(1, 3)]
                    }
                    // Sony 0x6044aecc explicitly accepts an immediate 4 as
                    // well as 2 -> confirm 3 -> 4. A parked/stale owner alone
                    // is insufficient: a live request must still exist.
                    4 if self.requesting => {
                        self.requesting = false;
                        self.granted = true;
                        let mut effects = Vec::new();
                        if let Some(value) = self.desired_volume.take() {
                            effects.push(owner.remote(3, value));
                        }
                        effects.extend([text(&format!("grant {}", owner.name())), text("audio on")]);
                        effects
                    }
                    1 | 5 => {
                        self.owner = None;
                        self.park()
                    }
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    pub fn timeout(&mut self) -> Vec<Effect> {
        self.owner = None;
        self.pending_ack = None;
        self.park()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_requires_controller_confirmation_before_audio_is_enabled() {
        let mut state = State::default();
        assert!(state.request(Source::Airplay).contains(&Effect::Remote([3, 1, 1, 0])));
        assert!(!state.granted);
        assert_eq!(state.incoming(0x40, &[3, 1, 1, 2]), vec![Effect::Remote([3, 1, 1, 3])]);
        assert!(!state.granted);
        assert!(state.incoming(0x40, &[3, 1, 1, 4]).contains(&text("audio on")));
        assert!(state.granted);
    }

    #[test]
    fn airplay_enable_ack_and_f9_preserve_the_pending_or_granted_session() {
        for direct in [false, true] {
            let mut state = State::default();
            state.request(Source::Airplay);
            assert_eq!(state.incoming(0x40, &[3, 0x0f, 1, 0x10]), vec![Effect::Remote([3, 0x0f, 1, 0x11])]);
            if !direct {
                assert_eq!(state.incoming(0x48, &[0xf9, 0]), vec![status(0xe3, 5)]);
                assert_eq!(state.incoming(0x40, &[3, 1, 1, 2]), vec![Effect::Remote([3, 1, 1, 3])]);
            }
            assert!(state.incoming(0x40, &[3, 1, 1, 4]).contains(&text("audio on")));
            state.playback(Source::Airplay, true);
            assert_eq!(state.incoming(0x48, &[0xf9, 0]), vec![status(0xe3, 5)]);
            assert!(state.granted && state.playing());
            let disable = state.incoming(0x40, &[3, 0x0f, 1, 0]);
            assert!(disable.contains(&text("audio off")) && disable.contains(&Effect::Remote([3, 0x0f, 1, 1])));
            assert!(state.incoming(0x40, &[3, 1, 1, 4]).is_empty());
        }
    }

    #[test]
    fn teardown_acknowledgement_waits_for_output_to_stop_and_sessions_to_close() {
        let mut state =
            State { owner: Some(Source::Airplay), granted: true, active: [true, false, false], ..State::default() };
        assert!(!state.incoming(0x48, &[0xf8, 0]).contains(&status(0xe3, 4)));
        assert_eq!(state.playback(Source::Airplay, false), vec![status(0xe3, 4)]);
        assert!(state.incoming(0x48, &[0xf7, 0]).contains(&status(0xe3, 0)));
        assert!(state.incoming(0x48, &[0xf9, 0]).contains(&Effect::Restart(Source::Airplay)));
        assert!(state.gone(Source::Airplay).contains(&status(0xe3, 5)));
        assert!(!state.granted);
    }

    #[test]
    fn a_non_owner_cannot_resume_audio_and_a_sleeping_request_announces_awake() {
        let mut state = State { sleeping: true, ..State::default() };
        assert!(state.request(Source::Airplay).contains(&status(0xe1, 4)));
        assert!(state.playback(Source::Spotify, true).is_empty());
        assert!(!state.playing());
        assert!(state.timeout().contains(&text("audio off")));
    }

    #[test]
    fn a_direct_ready_reply_is_valid_but_a_late_grant_after_parking_is_ignored() {
        let mut state = State::default();
        state.request(Source::Airplay);
        assert!(state.incoming(0x40, &[3, 1, 1, 4]).contains(&text("audio on")));
        state.incoming(0x48, &[0xf8, 0]);
        assert!(!state.granted);
        assert!(state.incoming(0x40, &[3, 1, 1, 4]).is_empty());
        assert!(!state.granted);
    }
}
