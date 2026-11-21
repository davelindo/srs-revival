#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    RhythmAndBlues,
    Flat,
    Rock,
    Pop,
    Jazz,
    Custom,
}

impl Preset {
    pub const ALL: [Self; 6] = [Self::RhythmAndBlues, Self::Flat, Self::Rock, Self::Pop, Self::Jazz, Self::Custom];

    pub fn name(self) -> &'static str {
        match self {
            Self::RhythmAndBlues => "R&B",
            Self::Flat => "Flat",
            Self::Rock => "Rock",
            Self::Pop => "Pop",
            Self::Jazz => "Jazz",
            Self::Custom => "Custom",
        }
    }

    pub fn code(self) -> u8 {
        self as u8
    }
}

pub const BANDS: [&str; 5] = ["100 Hz", "330 Hz", "1 kHz", "3.3 kHz", "10 kHz"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Up,
    Down,
    Mute(bool),
    ReadTuning,
    Eq(Preset),
    Bands([i8; 5]),
    Clear(bool),
    Wide(bool),
}

impl Action {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "read" => Self::Read,
            "up" => Self::Up,
            "down" => Self::Down,
            "mute" => Self::Mute(true),
            "unmute" => Self::Mute(false),
            "read-tuning" => Self::ReadTuning,
            "clear-on" => Self::Clear(true),
            "clear-off" => Self::Clear(false),
            "wide-high" => Self::Wide(true),
            "wide-normal" => Self::Wide(false),
            _ if text.starts_with("eq-") => Self::Eq(*Preset::ALL.get(text[3..].parse::<usize>().ok()?)?),
            _ if text.starts_with("bands-") => Self::Bands(parse_bands(&text[6..])?),
            _ => return None,
        })
    }

    pub fn tuning(self) -> bool {
        matches!(self, Self::ReadTuning | Self::Eq(_) | Self::Bands(_) | Self::Clear(_) | Self::Wide(_))
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn tuning_write(self) -> Option<Vec<u8>> {
        // Legacy SongPal SoundInfoSetReq (0x93), handled by Armada 0x58133ac8.
        Some(match self {
            Self::Eq(preset) => vec![0x93, 0x10, preset.code(), 0xff, 0, 0],
            Self::Bands(bands) => {
                let mut payload = vec![0x93, 0x10, 0x10, 0xff, 0x10, 10];
                for (index, gain) in bands.into_iter().enumerate() {
                    payload.extend([index as u8 + 1, (gain + 11) as u8]);
                }
                payload
            }
            Self::Clear(enabled) => vec![0x93, 0x11, 0xff, 0xff, 1, 1, u8::from(enabled)],
            Self::Wide(high) => vec![0x93, 0x20, 1, if high { 0x0f } else { 0x1f }, 0xff, 0, 0],
            _ => return None,
        })
    }
}

pub fn parse_bands(text: &str) -> Option<[i8; 5]> {
    let mut fields = text.split(',');
    let mut bands = [0; 5];
    for band in &mut bands {
        let level = fields.next()?.parse::<i8>().ok()?;
        if !(-10..=10).contains(&level) {
            return None;
        }
        *band = level;
    }
    fields.next().is_none().then_some(bands)
}

#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub struct Tuning {
    preset: Option<Preset>,
    bands: Option<[i8; 5]>,
    clear: Option<bool>,
    wide: Option<bool>,
}

#[cfg(any(target_os = "linux", test))]
impl Tuning {
    pub fn incoming(&mut self, payload: &[u8]) {
        // 0x58133430 publishes the active category in a SoundInfo (0x92) reply.
        match payload {
            [0x92, 0x10, preset @ 0..=5, 0xff, 0, _, _, 0, 0] => self.preset = Some(Preset::ALL[*preset as usize]),
            [0x92, 0x10, 0x10, 0xff, 0, _, 0, 0x10, 10, entries @ ..] if entries.len() == 10 => {
                let mut bands = [0; 5];
                for (index, entry) in entries.chunks_exact(2).enumerate() {
                    if entry[0] != index as u8 + 1 || !(1..=21).contains(&entry[1]) {
                        return;
                    }
                    bands[index] = entry[1] as i8 - 11;
                }
                self.bands = Some(bands);
            }
            [0x92, 0x11, 0xff, 0xff, 0, _, _, 1, 1, value @ (0 | 1)] => self.clear = Some(*value != 0),
            [0x92, 0x20, 1, value @ (0x0f | 0x1f), 0xff, 0, _, _, 0, 0] => self.wide = Some(*value == 0x0f),
            _ => {}
        }
    }

    pub fn report(&self, command: Action) -> Option<Result<String, &'static str>> {
        let (preset, clear, wide) = (self.preset?, self.clear?, self.wide?);
        let bands = if preset == Preset::Custom { Some(self.bands?) } else { None };
        let applied = match command {
            Action::Eq(value) => value == preset,
            Action::Bands(value) => Some(value) == bands,
            Action::Clear(value) => value == clear,
            Action::Wide(value) => value == wide,
            _ => true,
        };
        if !applied {
            return Some(Err("Sony reported a different sound setting"));
        }
        let mut report = format!("eq={} clear={} wide={}", preset.code(), u8::from(clear), u8::from(wide));
        if let Some([a, b, c, d, e]) = bands {
            report.push_str(&format!(" bands={a},{b},{c},{d},{e}"));
        }
        Some(Ok(report))
    }
}
