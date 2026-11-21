//! Small server-rendered management UI. No browser runtime or continuous
//! polling; the HTTP parser and HTML escaping come from maintained libraries.
use crate::Error;
use crate::sound::Action;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UnixStream},
    sync::Semaphore,
};

mod session;
#[cfg(test)]
mod tests;
mod view;

struct Web {
    state: PathBuf,
    settings: String,
    password: Vec<u8>,
    login_csrf: String,
    sessions: Mutex<session::Sessions>,
    controller: PathBuf,
}

fn random_text() -> Result<String, Error> {
    use std::fmt::Write;
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)?;
    let mut text = String::with_capacity(64);
    for byte in bytes {
        write!(text, "{byte:02x}")?;
    }
    Ok(text)
}

impl Web {
    fn recovery(&self) -> bool {
        zd1_sys::OsRelease::read("/etc/os-release").get("VARIANT_ID") == "rescue"
    }
    fn new(state: &str, settings: &str) -> Result<Self, Error> {
        fs::create_dir_all(state)?;
        let password_path = Path::new(state).join("web-password");
        let password = if password_path.exists() {
            fs::read_to_string(&password_path)?.trim().to_owned()
        } else {
            let password = random_text()?;
            zd1_sys::write_private(password_path.to_str().ok_or("invalid password path")?, &password)?;
            password
        };
        if !(16..=128).contains(&password.len()) || password.chars().any(char::is_control) {
            return Err("invalid management password file".into());
        }
        Ok(Self {
            state: state.into(),
            settings: settings.into(),
            password: password.into_bytes(),
            login_csrf: random_text()?,
            sessions: Mutex::new(session::Sessions::load(Path::new(state).join("web-sessions"))?),
            controller: "/run/zd1link".into(),
        })
    }

    fn values(&self) -> BTreeMap<String, String> {
        zd1_settings::load_from(&self.settings)
    }

    fn get(&self, key: &str) -> String {
        zd1_settings::get_from(&self.values(), key).unwrap_or_default()
    }

    fn network_dir(&self) -> &Path {
        Path::new(&self.settings).parent().unwrap_or(Path::new("/data/zd1"))
    }
}

pub async fn serve(listen: &str, state: &str, settings: &str) -> Result<(), Error> {
    let web = Arc::new(Web::new(state, settings)?);
    let listener = TcpListener::bind(listen).await?;
    let limit = Arc::new(Semaphore::new(4));
    eprintln!("Web interface listening on {}", listener.local_addr()?);
    #[cfg(all(target_os = "linux", target_arch = "arm"))]
    {
        let mut board = dm870::DevMem::open()?;
        // The rescue has only wired management. Binding 0.0.0.0 succeeds even if the PHY never comes up.
        // A confirmed playback system can also be used offline.
        if web.recovery() && dm870::mailbox::bl2_requested(&mut board) {
            loop {
                if zd1_net::Iface::open("eth0").is_ok_and(|iface| iface.carrier() && iface.ipv4().is_some()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        if dm870::complete_boot_recovery(&mut board) {
            eprintln!("Boot watchdog acknowledged");
        }
    }
    loop {
        let (mut socket, _) = listener.accept().await?;
        let Ok(permit) = limit.clone().try_acquire_owned() else {
            continue;
        };
        let web = web.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::time::timeout(Duration::from_secs(10), connection(&web, &mut socket)).await;
            if let Ok(Err(error)) = result {
                eprintln!("Web request: {error}");
            }
        });
    }
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
    token: Option<String>,
    form: bool,
    same_origin: bool,
}

async fn request(web: &Web, socket: &mut TcpStream) -> Result<Request, Error> {
    let mut bytes = Vec::with_capacity(2048);
    let (method, path, head_length, body_length, token, form, same_origin) = loop {
        let mut block = [0; 1024];
        let n = socket.read(&mut block).await?;
        if n == 0 {
            return Err("incomplete HTTP request".into());
        }
        bytes.extend_from_slice(&block[..n]);
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut parsed = httparse::Request::new(&mut headers);
        if let httparse::Status::Complete(head) = parsed.parse(&bytes)? {
            if head > 8192 {
                return Err("HTTP headers too large".into());
            }
            let mut length = None;
            let mut token = None;
            let mut form = false;
            let mut host = None;
            let mut origin = None;
            let mut cookie_seen = false;
            for header in parsed.headers {
                if header.name.eq_ignore_ascii_case("content-length") {
                    if length.is_some() {
                        return Err("duplicate content length".into());
                    }
                    length = Some(std::str::from_utf8(header.value)?.parse::<usize>()?);
                }
                if header.name.eq_ignore_ascii_case("transfer-encoding") {
                    return Err("chunked requests are unsupported".into());
                }
                if header.name.eq_ignore_ascii_case("cookie") {
                    if cookie_seen {
                        return Err("duplicate cookie header".into());
                    }
                    cookie_seen = true;
                    token = session::cookie(header.value);
                }
                if header.name.eq_ignore_ascii_case("host") && host.replace(header.value).is_some() {
                    return Err("duplicate host header".into());
                }
                if header.name.eq_ignore_ascii_case("origin") && origin.replace(header.value).is_some() {
                    return Err("duplicate origin header".into());
                }
                if header.name.eq_ignore_ascii_case("content-type") {
                    form =
                        header.value.split(|byte| *byte == b';').next() == Some(b"application/x-www-form-urlencoded");
                }
            }
            let length = length.unwrap_or(0);
            if length > 16384 {
                return Err("HTTP body too large".into());
            }
            break (
                parsed.method.unwrap_or("").to_owned(),
                parsed.path.unwrap_or("/").to_owned(),
                head,
                length,
                token.filter(|token| web.sessions.lock().unwrap().contains(&web.password, token)).map(str::to_owned),
                form,
                origin.is_none_or(|value| host.is_some_and(|host| value.strip_prefix(b"http://") == Some(host))),
            );
        }
        if bytes.len() >= 8192 {
            return Err("HTTP headers too large".into());
        }
    };
    let read_body = token.is_some() || (method == "POST" && path == "/login" && form && same_origin);
    if read_body {
        while bytes.len() < head_length + body_length {
            let mut block = [0; 1024];
            let remaining = (head_length + body_length - bytes.len()).min(block.len());
            let n = socket.read(&mut block[..remaining]).await?;
            if n == 0 {
                return Err("incomplete HTTP body".into());
            }
            bytes.extend_from_slice(&block[..n]);
        }
    }
    let body = if read_body { bytes[head_length..head_length + body_length].to_vec() } else { Vec::new() };
    Ok(Request { method, path, body, token, form, same_origin })
}

async fn reply(socket: &mut TcpStream, status: &str, headers: &str, body: &str) -> Result<(), Error> {
    response(socket, status, "text/html; charset=utf-8", headers, body).await
}

async fn response(
    socket: &mut TcpStream,
    status: &str,
    content_type: &str,
    headers: &str,
    body: &str,
) -> Result<(), Error> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'\r\n{headers}\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body.as_bytes()).await?;
    socket.shutdown().await?;
    Ok(())
}

async fn live_sound(socket: &mut TcpStream, result: Result<String, Error>) -> Result<(), Error> {
    let (status, body) = match result {
        Ok(report) => ("200 OK", report),
        Err(error) => ("409 Conflict", format!("Controller readback unavailable. {error}")),
    };
    response(socket, status, "text/plain; charset=utf-8", "", &body).await
}

async fn redirect(socket: &mut TcpStream, destination: &str, notice: &str) -> Result<(), Error> {
    let query = form_urlencoded::Serializer::new(String::new()).append_pair("notice", notice).finish();
    reply(socket, "303 See Other", &format!("Location: {destination}?{query}\r\n"), "").await
}

pub(crate) async fn init_request(line: &str) -> Result<String, Error> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut socket = UnixStream::connect(zd1_svc::SOCKET).await?;
        socket.write_all(format!("{line}\n").as_bytes()).await?;
        socket.shutdown().await?;
        let mut answer = Vec::new();
        socket.take(16384).read_to_end(&mut answer).await?;
        if !answer.starts_with(b"ok") {
            return Err("service restart was not acknowledged".into());
        }
        Ok::<_, Error>(String::from_utf8(answer)?)
    })
    .await?
}

async fn controller_reply(socket_path: &Path, line: &str, prefix: &str, timeout: Duration) -> Result<String, Error> {
    tokio::time::timeout(timeout, async {
        let mut socket = UnixStream::connect(socket_path).await?;
        socket.write_all(format!("{line}\n").as_bytes()).await?;
        let mut response = Vec::new();
        let mut bytes = [0; 256];
        let mut received = 0;
        loop {
            let count = socket.read(&mut bytes).await?;
            received += count;
            if count == 0 || received > 8192 {
                return Err("Controller service did not acknowledge the request".into());
            }
            response.extend_from_slice(&bytes[..count]);
            while let Some(end) = response.iter().position(|byte| *byte == b'\n') {
                let line = std::str::from_utf8(&response[..end])?;
                if let Some(result) = line.strip_prefix(prefix) {
                    if let Some(error) = result.strip_prefix("error ") {
                        return Err(error.to_owned().into());
                    }
                    return Ok::<_, Error>(result.to_owned());
                }
                response.drain(..=end);
            }
        }
    })
    .await?
}

async fn sound_request(web: &Web, action: &str) -> Result<String, Error> {
    let mut id = [0; 4];
    getrandom::fill(&mut id)?;
    let id = u32::from_ne_bytes(id);
    controller_reply(&web.controller, &format!("sound {id} {action}"), &format!("sound {id} "), Duration::from_secs(6))
        .await
}

const SETTINGS: &[&str] = &[
    "name",
    "airplay",
    "airplay.pin",
    "spotify",
    "spotify.bitrate",
    "snapcast",
    "snapcast.server",
    "snapcast.latency",
    "eth.mode",
    "eth.address",
    "eth.gateway",
    "eth.dns",
    "wifi.mode",
    "wifi.address",
    "wifi.gateway",
    "wifi.dns",
];

async fn connection(web: &Web, socket: &mut TcpStream) -> Result<(), Error> {
    let request = request(web, socket).await?;
    if request.path == "/login" {
        return login(web, socket, request).await;
    }
    let Some(token) = request.token else {
        return reply(socket, "401 Unauthorized", "", &view::login(web, None)).await;
    };
    let form_token = session::csrf(&token);
    let (path, query) = request.path.split_once('?').unwrap_or((&request.path, ""));
    let sound_page = matches!(path, "/sound" | "/tuning");
    let live = sound_page && query == "live=1";
    if request.method == "GET" {
        if path == "/dashboard.js" {
            return response(socket, "200 OK", "text/javascript; charset=utf-8", "", include_str!("web/dashboard.js"))
                .await;
        }
        if live {
            let result = if web.recovery() {
                Err("Sound controls are unavailable in recovery".into())
            } else {
                sound_request(web, if path == "/tuning" { "read-tuning" } else { "read" }).await
            };
            return live_sound(socket, result).await;
        }
        if !["/", "/sound", "/tuning", "/system", "/network", "/power", "/ssh", "/debug"].contains(&path) {
            return reply(socket, "404 Not Found", "", "Page not found.").await;
        }
        let notice = form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "notice")
            .map(|(_, value)| value.into_owned());
        let detail = match path {
            "/debug" => Some(
                init_request("status").await.unwrap_or_else(|error| format!("Service status unavailable: {error}")),
            ),
            "/sound" | "/tuning" if !web.recovery() => Some(
                sound_request(web, if path == "/tuning" { "read-tuning" } else { "read" })
                    .await
                    .unwrap_or_else(|error| format!("Controller readback unavailable: {error}")),
            ),
            _ => None,
        };
        return reply(socket, "200 OK", "", &view::page(web, &form_token, path, notice.as_deref(), detail.as_deref()))
            .await;
    }
    if request.method != "POST" {
        return reply(socket, "405 Method Not Allowed", "Allow: GET, POST\r\n", "Unsupported method.").await;
    }
    if !request.form {
        return reply(socket, "415 Unsupported Media Type", "", "Use a form submission.").await;
    }
    let mut fields: BTreeMap<String, String> = form_urlencoded::parse(&request.body).into_owned().collect();
    let csrf = fields.remove("_csrf").unwrap_or_default();
    if !request.same_origin || !bool::from(csrf.as_bytes().ct_eq(form_token.as_bytes())) {
        return reply(socket, "403 Forbidden", "", "Form expired. Reload the page.").await;
    }
    if path == "/logout" {
        web.sessions.lock().unwrap().revoke(&web.password, &token)?;
        let headers = format!(
            "Location: /login\r\nSet-Cookie: {}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0\r\n",
            session::COOKIE
        );
        return reply(socket, "303 See Other", &headers, "").await;
    }
    let destination = match fields.remove("_return").as_deref() {
        Some("/network") => "/network",
        Some("/ssh") => "/ssh",
        Some("/power") => "/power",
        Some("/sound") => "/sound",
        Some("/system") => "/system",
        _ => "/",
    };
    if sound_page {
        let action = if path == "/tuning" && fields.len() == 5 {
            let bands = ["band-1", "band-2", "band-3", "band-4", "band-5"];
            let values: Vec<_> = bands.iter().map(|key| fields.get(*key).map_or("", String::as_str)).collect();
            Some(format!("bands-{}", values.join(",")))
        } else if fields.len() == 1 {
            fields.remove("action")
        } else {
            None
        };
        let valid = action.and_then(|text| {
            let action = Action::parse(&text)?;
            (action.tuning() == (path == "/tuning") && !matches!(action, Action::Read | Action::ReadTuning))
                .then_some(text)
        });
        let Some(action) = valid.filter(|_| !web.recovery()) else {
            return reply(socket, "400 Bad Request", "", "Select a valid sound setting.").await;
        };
        let result = sound_request(web, &action).await;
        if live {
            return live_sound(socket, result).await;
        }
        match result {
            Ok(_) => return redirect(socket, path, "Sony controller returned the updated sound status.").await,
            Err(error) => {
                let notice = format!("Sony controller did not confirm the command. {error}");
                return reply(socket, "409 Conflict", "", &view::page(web, &form_token, path, Some(&notice), None))
                    .await;
            }
        }
    }
    let result = match path {
        "/settings" => {
            if fields.keys().any(|key| !SETTINGS.contains(&key.as_str())) {
                Err("Unknown setting".into())
            } else {
                let values: Vec<_> = fields.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
                zd1_settings::set_many_in(&web.settings, &values).map_err(Into::into)
            }
        }
        "/wifi" => wifi(web, &fields),
        "/keys" => save_keys(web, fields.get("keys").map(String::as_str).unwrap_or("")),
        "/power" => {
            if fields.keys().any(|key| !matches!(key.as_str(), "network" | "automatic")) {
                Err("Unknown standby setting".into())
            } else {
                crate::power::save(
                    &web.settings,
                    fields.get("network").map_or("", String::as_str),
                    fields.get("automatic").map_or("", String::as_str),
                )
            }
        }
        _ => Err("Unknown action".into()),
    };
    if let Err(error) = result {
        return reply(
            socket,
            "400 Bad Request",
            "",
            &view::page(web, &form_token, destination, Some(&error.to_string()), None),
        )
        .await;
    }
    let affected = match path {
        "/wifi" => "wifi.add".into(),
        "/keys" => "ssh.keys".into(),
        _ => fields.keys().cloned().collect::<Vec<_>>().join(" "),
    };
    let notice = if path == "/power" {
        match controller_reply(&web.controller, "power apply", "power queued", Duration::from_secs(2)).await {
            Ok(_) => "Settings saved. Waiting for controller readback; reload to see the result.",
            Err(_) => "Settings saved. Controller service is unavailable; see Diagnostics.",
        }
    } else {
        match init_request(&format!("changed {affected}")).await {
            Ok(_) if web.recovery() => "Settings saved for the next system boot.",
            Ok(_) => "Changes saved. Services will apply them shortly.",
            Err(error) => {
                eprintln!("Settings apply: {error}");
                "Changes saved. Service restart needs attention; see Diagnostics."
            }
        }
    };
    redirect(socket, destination, notice).await
}

async fn login(web: &Web, socket: &mut TcpStream, request: Request) -> Result<(), Error> {
    if request.method == "GET" {
        if request.token.is_some() {
            return reply(socket, "303 See Other", "Location: /\r\n", "").await;
        }
        return reply(socket, "200 OK", "", &view::login(web, None)).await;
    }
    if request.method != "POST" {
        return reply(socket, "405 Method Not Allowed", "Allow: GET, POST\r\n", "Unsupported method.").await;
    }
    if !request.form || !request.same_origin {
        return reply(socket, "403 Forbidden", "", "Open the sign-in page on the speaker.").await;
    }
    let fields: BTreeMap<_, _> = form_urlencoded::parse(&request.body).collect();
    let csrf = fields.get("_csrf").map_or("", |value| value.as_ref());
    if !bool::from(csrf.as_bytes().ct_eq(web.login_csrf.as_bytes())) {
        return reply(socket, "403 Forbidden", "", &view::login(web, Some("Reload the sign-in page and try again.")))
            .await;
    }
    let password = fields.get("password").map_or("", |value| value.as_ref());
    if !bool::from(password.as_bytes().ct_eq(&web.password)) {
        tokio::time::sleep(Duration::from_millis(350)).await;
        return reply(socket, "401 Unauthorized", "", &view::login(web, Some("Check the speaker password."))).await;
    }
    let token = random_text()?;
    let cookie = web.sessions.lock().unwrap().issue(&web.password, &token)?;
    reply(socket, "303 See Other", &format!("Location: /\r\n{cookie}\r\n"), "").await
}

fn wifi(web: &Web, fields: &BTreeMap<String, String>) -> Result<(), Error> {
    let ssid = fields.get("ssid").map(String::as_str).unwrap_or("");
    let password = fields.get("password").map(String::as_str).unwrap_or("");
    if ssid.is_empty() || ssid.len() > 32 || ssid.chars().any(char::is_control) {
        return Err("Network name must be 1–32 bytes without control characters.".into());
    }
    let network = if fields.get("open").is_some_and(|value| value == "on") {
        format!("open {ssid}")
    } else {
        if !(8..=63).contains(&password.len()) || password.chars().any(char::is_control) {
            return Err("WPA2 passphrase must be 8–63 bytes.".into());
        }
        let mut key = [0; 32];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), ssid.as_bytes(), 4096, &mut key);
        let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("wpa2 {hex} {ssid}")
    };
    zd1_settings::act_in(web.network_dir(), "wifi.add", &network).map_err(Into::into)
}

fn save_keys(web: &Web, text: &str) -> Result<(), Error> {
    if text.len() > 8192 {
        return Err("Public keys exceed 8 KiB.".into());
    }
    let lines: Vec<_> = text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).collect();
    if lines.is_empty() || lines.len() > 16 {
        return Err("Provide between 1 and 16 Ed25519 public keys.".into());
    }
    for line in &lines {
        let key = russh::keys::PublicKey::from_openssh(line).map_err(|_| "Invalid SSH public key.")?;
        if key.algorithm() != russh::keys::Algorithm::Ed25519 {
            return Err("Use Ed25519 public keys.".into());
        }
    }
    zd1_sys::write_private(
        web.state.join("authorized_keys").to_str().ok_or("invalid key path")?,
        &format!("{}\n", lines.join("\n")),
    )?;
    Ok(())
}
