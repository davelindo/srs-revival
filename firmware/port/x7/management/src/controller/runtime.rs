//! One reactor owns the controller state. SPI transactions stay in the kernel;
//! only completed controller writes, local receiver commands and network route
//! changes wake this process. Timers exist only for active handshakes/recovery.
use super::startup;
use super::state::{Effect, Source, State};
use crate::Error;
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io,
    net::Ipv4Addr,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::PermissionsExt,
    },
    path::Path,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, unix::AsyncFd},
    net::{UnixListener, UnixStream},
    sync::{broadcast, mpsc},
    time::Instant,
};
use x7_control::{IOCTL, NOTIFY, POP_WRITE, Packet, SET_FIXED, SET_VARIABLE};

include!(concat!(env!("OUT_DIR"), "/sony-fixed.rs"));
const SOCKET: &str = "/run/zd1link";
const NATIVE_TIMEOUT: Duration = Duration::from_secs(5);

fn transfer(file: &impl AsRawFd, packet: &mut Packet) -> io::Result<()> {
    if unsafe { libc::ioctl(file.as_raw_fd(), IOCTL as _, packet as *mut Packet) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn packet(operation: u8, register: u8, variable: bool, bytes: &[u8]) -> Result<Packet, Error> {
    let mut packet = Packet::new(operation);
    packet.register = register;
    packet.kind = if variable { 3 } else { 1 };
    if !packet.set_data(bytes) {
        return Err("controller value exceeds 512 bytes".into());
    }
    Ok(packet)
}

fn insert_driver(bytes: &[u8]) -> io::Result<()> {
    let result = unsafe { libc::syscall(libc::SYS_init_module, bytes.as_ptr(), bytes.len(), c"".as_ptr()) };
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn load_driver(image: &str, path: &str) -> Result<(), Error> {
    let bytes = fs::read(image)?;
    if bytes.len() > 1024 * 1024 {
        return Err("SPI kernel module exceeds 1 MiB".into());
    }
    let installed = match insert_driver(&bytes) {
        Ok(()) => true,
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
            let version = match fs::read_to_string("/sys/module/x7_spi/version") {
                Ok(version) => version,
                Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
                Err(error) => return Err(error.into()),
            };
            if format!("version={}\0", version.trim()).as_bytes() == x7_control::DRIVER_MODINFO {
                false
            } else {
                // The old rescue loaded its driver before switch_root. At this point PID 1 has
                // started no services. Never force removal of a module held by a live client.
                let interrupts = fs::read_to_string("/proc/interrupts")?;
                if let Some(line) = interrupts.lines().find(|line| line.split_whitespace().last() == Some("x7-spi")) {
                    eprintln!("Sony SPI before driver handoff: {}", line.trim());
                    if let Err(error) = zd1_sys::write_private("/run/zd1/sony-spi-before-slot", line.trim()) {
                        eprintln!("Could not retain rescue SPI interrupt count: {error}");
                    }
                }
                if unsafe { libc::syscall(libc::SYS_delete_module, c"x7_spi".as_ptr(), libc::O_NONBLOCK) } != 0 {
                    return Err(io::Error::last_os_error().into());
                }
                insert_driver(&bytes)?;
                true
            }
        }
        Err(error) => return Err(error.into()),
    };
    let device = open_device(path)?;
    if installed {
        transfer(&device, &mut packet(SET_VARIABLE, 0x25, true, b"srs-x7-recovery\0")?)?;
    }
    eprintln!("Sony SPI driver ready at {path}");
    Ok(())
}

fn open_device(path: &str) -> Result<File, Error> {
    let open = || -> io::Result<File> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    };
    match open() {
        Ok(device) => return Ok(device),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let minor: u32 = fs::read_to_string("/proc/misc")?
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            let minor = words.next()?.parse().ok()?;
            (words.next()? == "x7-spi").then_some(minor)
        })
        .ok_or("x7-spi did not register its misc device")?;
    match zd1_sys::mknod(path, libc::S_IFCHR, 0o600, 10, minor) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    Ok(open()?)
}

async fn incoming(device: &AsyncFd<File>) -> Result<Packet, Error> {
    loop {
        let mut ready = device.readable().await?;
        match ready.try_io(|device| {
            let mut packet = Packet::new(POP_WRITE);
            transfer(device.get_ref(), &mut packet)?;
            Ok(packet)
        }) {
            Ok(result) => return Ok(result?),
            Err(_) => continue,
        }
    }
}

fn routes() -> io::Result<AsyncFd<OwnedFd>> {
    // SOCK_NONBLOCK/SOCK_CLOEXEC were added after this kernel: set both flags
    // through fcntl, as in the tested PTY backend.
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as _;
    address.nl_groups = (libc::RTMGRP_LINK | libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV4_ROUTE) as _;
    if unsafe {
        libc::bind(fd.as_raw_fd(), (&address as *const libc::sockaddr_nl).cast(), std::mem::size_of_val(&address) as _)
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // The OwnedFd is never replaced or closed while the registration exists.
    Ok(unsafe { AsyncFd::register(fd)? })
}

async fn route_change(route: &AsyncFd<OwnedFd>) -> io::Result<()> {
    loop {
        let mut ready = route.readable().await?;
        match ready.try_io(|route| {
            let mut bytes = [0u8; 4096];
            let count = unsafe { libc::recv(route.get_ref().as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len(), 0) };
            if count < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
        }) {
            Ok(result) => return result,
            Err(_) => continue,
        }
    }
}

enum App {
    Line(u64, String),
    Gone(u64),
}

async fn client(
    mut stream: UnixStream,
    id: u64,
    tx: mpsc::Sender<App>,
    mut events: broadcast::Receiver<String>,
    initial: String,
) {
    if stream.write_all(initial.as_bytes()).await.is_err() {
        let _ = tx.send(App::Gone(id)).await;
        return;
    }
    let mut line = Vec::with_capacity(128);
    let mut bytes = [0; 256];
    'connection: loop {
        tokio::select! {
            read = stream.read(&mut bytes) => {
                let Ok(n) = read else { break; };
                if n == 0 { break; }
                for &byte in &bytes[..n] {
                    if byte == b'\n' {
                        let Ok(text) = String::from_utf8(std::mem::take(&mut line)) else { break 'connection; };
                        if tx.send(App::Line(id, text)).await.is_err() { break 'connection; }
                    } else {
                        if line.len() == 512 { break 'connection; }
                        line.push(byte);
                    }
                }
            }
            event = events.recv() => {
                let Ok(event) = event else { break; };
                if stream.write_all(event.as_bytes()).await.is_err() || stream.write_all(b"\n").await.is_err() { break; }
            }
        }
    }
    let _ = tx.send(App::Gone(id)).await;
}

struct Actor {
    device: AsyncFd<File>,
    state: State,
    events: broadcast::Sender<String>,
    notes: VecDeque<Packet>,
    clients: Vec<(u64, Option<Source>)>,
    handshake: Option<Instant>,
    recovery: Option<Instant>,
    address: String,
    settings: String,
    power: super::power_link::Link,
    power_deadline: Option<Instant>,
    sound_client: Option<(u32, Instant, crate::sound::Action)>,
}

impl Actor {
    fn power_send(&mut self, messages: Vec<Vec<u8>>) -> Result<(), Error> {
        if !messages.is_empty() {
            self.power_deadline = Some(Instant::now() + NATIVE_TIMEOUT);
        }
        for message in messages {
            self.notify(0x40, &message, true)?;
        }
        let status = if self.power.complete() {
            self.power_deadline = None;
            "Confirmed"
        } else {
            "Waiting for controller"
        };
        zd1_sys::write_private(crate::power::REPORT, &self.power.report(status))?;
        Ok(())
    }
    fn set(&self, register: u8, bytes: &[u8], variable: bool) -> Result<(), Error> {
        transfer(
            self.device.get_ref(),
            &mut packet(if variable { SET_VARIABLE } else { SET_FIXED }, register, variable, bytes)?,
        )?;
        Ok(())
    }
    fn notify(&mut self, register: u8, bytes: &[u8], variable: bool) -> Result<(), Error> {
        self.set(register, bytes, variable)?;
        if self.notes.len() == 32 {
            return Err("controller notification backlog exceeded 32 messages".into());
        }
        self.notes.push_back(packet(NOTIFY, register, variable, bytes)?);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        while let Some(note) = self.notes.front_mut() {
            match transfer(self.device.get_ref(), note) {
                Ok(()) => {
                    self.notes.pop_front();
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    async fn effects(&mut self, effects: Vec<Effect>) -> Result<(), Error> {
        for effect in effects {
            match effect {
                Effect::Fixed(register, value) => self.notify(register, &value, false)?,
                Effect::Remote(bytes) => {
                    eprintln!("Sony source command to controller: {bytes:02x?}");
                    if bytes[1] == 1 && matches!(bytes[3], 0 | 3) {
                        self.handshake = Some(Instant::now() + Duration::from_secs(5));
                    }
                    self.notify(0x40, &bytes, true)?;
                }
                Effect::Broadcast(line) => {
                    let _ = self.events.send(line);
                }
                Effect::Restart(source) => {
                    crate::web::init_request(&format!("restart {}", source.name())).await?;
                }
                Effect::Recovery => {
                    eprintln!("Sony controller requested updater entry on register 28");
                    for source in ["airplay", "spotify", "snapcast"] {
                        crate::web::init_request(&format!("stop {source}")).await?;
                    }
                    self.recovery = Some(Instant::now() + Duration::from_secs(4));
                }
            }
        }
        Ok(())
    }
    fn net(&mut self) -> Result<(), Error> {
        let ethernet = zd1_net::Iface::open("eth0").ok();
        let wired = ethernet.as_ref().filter(|interface| interface.carrier()).and_then(|interface| interface.ipv4());
        let wifi = zd1_net::Iface::open("wlan0").ok().and_then(|interface| interface.ipv4());
        let address = wired
            .filter(|ip| !ip.is_link_local())
            .or(wifi)
            .or(wired)
            .filter(|ip| *ip != Ipv4Addr::UNSPECIFIED)
            .map(|ip| ip.to_string())
            .unwrap_or_default();
        if address != self.address {
            self.address = address;
            let mut bytes = self.address.as_bytes().to_vec();
            bytes.push(0);
            self.notify(0x38, &bytes, true)?;
            self.notify(0x32, if self.address.is_empty() { b"00" } else { b"21" }, false)?;
            let _ = self.events.send(if self.address.is_empty() {
                "net down".into()
            } else {
                format!("net up {}", self.address)
            });
        }
        Ok(())
    }
    async fn app(&mut self, app: App) -> Result<(), Error> {
        let effects = match app {
            App::Gone(id) => {
                let source = self.clients.iter().find(|(client, _)| *client == id).and_then(|(_, source)| *source);
                self.clients.retain(|(client, _)| *client != id);
                source.map_or_else(Vec::new, |source| self.state.gone(source))
            }
            App::Line(id, line) => {
                let words: Vec<_> = line.split_whitespace().collect();
                let source = words.last().and_then(|name| Source::parse(name));
                if let Some(source) = source
                    && let Some(client) = self.clients.iter_mut().find(|(client, _)| *client == id)
                {
                    client.1 = Some(source);
                }
                match words.as_slice() {
                    ["sound", token, action] => {
                        if let (Ok(token), Some(action)) = (token.parse::<u32>(), crate::sound::Action::parse(action)) {
                            let now = Instant::now();
                            let result = if self.sound_client.is_some_and(|(_, deadline, _)| deadline > now) {
                                Err("sound request in progress")
                            } else {
                                self.sound_client = None;
                                self.power.sound(action)
                            };
                            match result {
                                Ok(messages) => {
                                    self.sound_client = Some((token, now + Duration::from_secs(5), action));
                                    self.power_send(messages)?;
                                }
                                Err(reason) => {
                                    let _ = self.events.send(format!("sound {token} error {reason}"));
                                }
                            }
                        }
                        Vec::new()
                    }
                    ["power", "apply"] => {
                        let messages = self.power.apply(crate::power::load(&self.settings));
                        self.power_send(messages)?;
                        let _ = self.events.send("power queued".into());
                        Vec::new()
                    }
                    ["intent", name] => {
                        if let Some(source) = Source::parse(name) {
                            self.state.request(source)
                        } else {
                            Vec::new()
                        }
                    }
                    ["state", playing, name] => Source::parse(name)
                        .map_or_else(Vec::new, |source| self.state.playback(source, *playing == "playing")),
                    ["volume", value] => {
                        value.parse::<u8>().ok().map_or_else(Vec::new, |value| self.state.volume(value))
                    }
                    ["session", "off", name] => {
                        Source::parse(name).map_or_else(Vec::new, |source| self.state.playback(source, false))
                    }
                    _ => Vec::new(),
                }
            }
        };
        if self.state.owner.is_none() {
            self.handshake = None;
        }
        self.effects(effects).await
    }
}

pub async fn serve(path: &str, settings: &str) -> Result<(), Error> {
    let file = open_device(path)?;
    let startup = startup::Settings::load();
    // File ownership remains stable until the reactor registration is dropped.
    let device = unsafe { AsyncFd::register(file)? };
    let route = routes()?;
    let (events, _) = broadcast::channel(16);
    let (tx, mut rx) = mpsc::channel(16);
    let mut actor = Actor {
        device,
        state: State::default(),
        events,
        notes: VecDeque::new(),
        clients: Vec::new(),
        handshake: None,
        recovery: None,
        address: String::new(),
        settings: settings.into(),
        power: super::power_link::Link::default(),
        power_deadline: None,
        sound_client: None,
    };
    for &(register, value) in FIXED {
        if register != 0x11 {
            let value = startup.initial_value(register, value);
            actor.set(register, &value, false)?;
        }
    }
    for (register, value) in [
        (0x25, &b"srs-x7-linux\0"[..]),
        (0x26, &b"srs-x7-boot\0"[..]),
        (0x27, &b"linux-settings-v1\0"[..]),
        // There is no Sony HLDC software image in this replacement. The MC's
        // 0x58156644 parser explicitly handles a short value as unknown.
        (0x90, &b"\0"[..]),
        (0x38, &b"\0"[..]),
        (0x39, &b"\0"[..]),
        (0x3a, &b"\0"[..]),
        (0x3e, &b"\0"[..]),
        (0x3f, &b"\0"[..]),
    ] {
        actor.set(register, value, true)?;
    }
    let name = zd1_settings::get_from(&zd1_settings::load_from(settings), "name").unwrap_or_else(|| "SRS-X7".into());
    actor.set(0x58, &format!("{name}\0").into_bytes(), true)?;
    let mac = zd1_persparam::unit_mac()?;
    let renderer = super::renderer_identity(mac);
    let mac = mac.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(":");
    actor.set(0x33, &format!("{mac}\0").into_bytes(), true)?;
    actor.set(0x32, b"00", false)?;
    actor.set(0xe3, &[5, 0], false)?;
    actor.set(0xe1, &[4, 0], false)?;
    if Path::new(SOCKET).exists() {
        fs::remove_file(SOCKET)?;
    }
    let listener = UnixListener::bind(SOCKET)?;
    fs::set_permissions(SOCKET, fs::Permissions::from_mode(0o600))?;
    // PID 1 starts linkd only after it has selected a root. During the
    // handoff the IRQ bank answers 00 00. Announce the root we actually run.
    let release = zd1_sys::OsRelease::read("/etc/os-release");
    let status = if release.get("VARIANT_ID") == "rescue" { 0x20 } else { 0x21 };
    actor.notify(0x11, &[status, 0], false)?;
    actor.notify(0xdc, &renderer, true)?;
    actor.notify(0x25, b"srs-x7-linux\0", true)?;
    actor.net()?;
    eprintln!("Sony controller link ready on {SOCKET}");
    let messages = actor.power.begin(crate::power::load(settings));
    actor.power_send(messages)?;
    let mut next_id = 0u64;
    loop {
        actor.flush()?;
        let handshake = actor.handshake;
        let recovery = actor.recovery;
        let power_deadline = actor.power_deadline;
        tokio::select! {
            message = incoming(&actor.device) => {
                let message = message?;
                let bytes = message.payload().ok_or("invalid controller packet length")?;
                // The local renderer receives five-byte operation replies and variable-length
                // source reports. Keep native SongPal frames out of the source trace.
                let local_renderer = matches!(bytes, [1, 1, 2, _, _] | [1, 2, 3, _, _, _] | [1, 5, _, ..]);
                if (message.register == 0x40 && (bytes.len() == 4 || local_renderer)) || message.register == 0x48 {
                    eprintln!("Sony source command from controller: register {:02x}, {bytes:02x?}", message.register);
                } else if let (0x81, [command, argument]) = (message.register, bytes) {
                    eprintln!("Controller write: 81, 2 bytes [{command:02x}, {argument:02x}]");
                } else {
                    eprintln!("Controller write: {:02x}, {} bytes", message.register, bytes.len());
                }
                if let Some((label, reply)) = startup.fixed_reply(message.register, bytes) {
                    actor.notify(message.register, &reply, false)?;
                    eprintln!("{label}: {reply:02x?}");
                    continue;
                }
                if message.register == 0x7e
                    && let Some(reply) = startup::reply(bytes, |name, value| {
                        zd1_sys::write_private(&format!("/run/zd1/{name}"), value)
                    })
                {
                    eprintln!("Sony startup 7e: {:02x?} ({} reply bytes)", &reply[..2], reply.len());
                    actor.notify(0x7e, &reply, true)?;
                    continue;
                }
                if message.register == 0x40 && bytes.starts_with(&[1, 0x21]) {
                    let replies = actor.power.incoming(bytes);
                    actor.power_send(replies)?;
                    if let Some((token, deadline, action)) = actor.sound_client
                        && Instant::now() <= deadline
                        && let Some(result) = actor.power.sound_result(action)
                    {
                        actor.sound_client = None;
                        let report = result.unwrap_or_else(|error| format!("error {error}"));
                        let _ = actor.events.send(format!("sound {token} {report}"));
                    }
                    continue;
                }
                if message.register == 0x40 && bytes == [1, 0x23, 1, 0] {
                    actor.power.channel_closed();
                    actor.power_deadline = actor.power.reconnect_pending().then(|| Instant::now() + NATIVE_TIMEOUT);
                    actor.notify(0x40, &[1, 0x22, 1, 0], true)?;
                    zd1_sys::write_private(crate::power::REPORT, &actor.power.report("Controller closed the status session"))?;
                    continue;
                }
                let had_grant = actor.state.granted;
                let effects = actor.state.incoming(message.register, bytes);
                // Armada can grant a network source directly from standby
                // without an explicit PSM exit command.
                let woke = message.register == 0xe1 && bytes.first() == Some(&2);
                if (woke || (!had_grant && actor.state.granted)) && actor.power.can_retry() {
                    actor.power_deadline = Some(Instant::now() + NATIVE_TIMEOUT);
                }
                if actor.state.granted || actor.state.owner.is_none() { actor.handshake = None; }
                actor.effects(effects).await?;
            }
            app = rx.recv() => { if let Some(app) = app { actor.app(app).await?; } }
            result = route_change(&route) => { result?; actor.net()?; }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                if actor.clients.len() >= 8 { continue; }
                next_id = next_id.wrapping_add(1);
                actor.clients.push((next_id, None));
                let mut initial = "audio off\nyield sony\n".to_string();
                if let Some(volume) = actor.state.volume { initial += &format!("volume {volume}\n"); }
                initial += &if actor.address.is_empty() { "net down\n".into() } else { format!("net up {}\n", actor.address) };
                tokio::spawn(client(stream, next_id, tx.clone(), actor.events.subscribe(), initial));
            }
            result = async { let mut ready = actor.device.writable().await?; ready.clear_ready(); Ok::<_, io::Error>(()) }, if !actor.notes.is_empty() => { result?; }
            _ = async { if let Some(deadline) = handshake { tokio::time::sleep_until(deadline).await; } else { std::future::pending::<()>().await; } } => {
                actor.handshake = None;
                let effects = actor.state.timeout();
                actor.effects(effects).await?;
                eprintln!("Controller source request timed out; output remains parked");
            }
            _ = async { if let Some(deadline) = recovery { tokio::time::sleep_until(deadline).await; } else { std::future::pending::<()>().await; } } => {
                unsafe { libc::sync(); }
                let mut bus = dm870::DevMem::open()?;
                eprintln!("Resetting DM870 into BL2 at the Sony controller's request");
                dm870::reset_into_bl2(&mut bus);
            }
            _ = async { if let Some(deadline) = power_deadline { tokio::time::sleep_until(deadline).await; } else { std::future::pending::<()>().await; } } => {
                actor.power_deadline = None;
                let closed = actor.power.reconnect_pending();
                if !actor.state.sleeping
                    && let Some(messages) = actor.power.retry_after_timeout()
                {
                    actor.power_send(messages)?;
                    let reason = if closed { "controller close" } else { "startup timeout" };
                    eprintln!("Reopened Sony native status session after {reason}");
                } else {
                    zd1_sys::write_private(crate::power::REPORT, &actor.power.report("Controller did not confirm the requested settings"))?;
                }
            }
        }
    }
}
