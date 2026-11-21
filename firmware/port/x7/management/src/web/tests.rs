use super::*;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("x7-web-{}", &random_text().unwrap()[..16]));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn web(&self) -> Web {
        let mut web = Web::new(self.0.to_str().unwrap(), self.0.join("settings").to_str().unwrap()).unwrap();
        web.controller = self.0.join("controller.sock");
        web
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn exchange(web: &Web, method: &str, path: &str, headers: &str, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let (mut server, _) = listener.accept().await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: speaker.test\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n{headers}\r\n{body}",
        body.len()
    );
    let (handled, response) = tokio::join!(connection(web, &mut server), async {
        client.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        response
    });
    handled.unwrap();
    response
}

fn status(reply: &str, expected: u16) {
    assert!(reply.starts_with(&format!("HTTP/1.1 {expected} ")), "{}", reply.lines().next().unwrap_or("no response"));
}

async fn sign_in(web: &Web) -> String {
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("_csrf", &web.login_csrf)
        .append_pair("password", std::str::from_utf8(&web.password).unwrap())
        .finish();
    let response = exchange(web, "POST", "/login", "Origin: http://speaker.test\r\n", &form).await;
    status(&response, 303);
    let cookie = response.lines().find_map(|line| line.strip_prefix("Set-Cookie: ")).unwrap();
    assert!(cookie.contains("; HttpOnly; SameSite=Strict; Max-Age=2592000"));
    cookie.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn cookie_login_survives_restart_and_logout_revokes_it() {
    let fixture = Fixture::new();
    let web = fixture.web();
    let response = exchange(&web, "GET", "/", "", "").await;
    status(&response, 401);
    assert!(response.contains("name=\"password\""));
    assert!(!response.contains("WWW-Authenticate:"));
    status(&exchange(&web, "POST", "/login", "", "password=invalid").await, 403);

    let cookie = sign_in(&web).await;
    let token = session::cookie(cookie.as_bytes()).unwrap();
    let ledger = fs::read_to_string(fixture.0.join("web-sessions")).unwrap();
    assert!(!ledger.contains(token));
    let headers = format!("Cookie: {cookie}\r\n");
    let csrf = session::csrf(token);
    drop(web);
    let web = fixture.web();
    status(&exchange(&web, "GET", "/", &headers, "").await, 200);
    status(&exchange(&web, "POST", "/logout", &headers, "").await, 403);
    let form = format!("_csrf={csrf}");
    let cross_site = format!("{headers}Origin: http://elsewhere.test\r\n");
    status(&exchange(&web, "POST", "/logout", &cross_site, &form).await, 403);
    status(&exchange(&web, "GET", "/", &headers, "").await, 200);
    let logout = exchange(&web, "POST", "/logout", &headers, &form).await;
    status(&logout, 303);
    assert!(logout.contains("Max-Age=0"));
    drop(web);
    status(&exchange(&fixture.web(), "GET", "/", &headers, "").await, 401);
}

#[tokio::test]
async fn sound_forms_send_only_validated_actions_and_require_a_controller_reply() {
    let fixture = Fixture::new();
    let web = fixture.web();
    let cookie = sign_in(&web).await;
    let headers = format!("Cookie: {cookie}\r\n");
    let csrf = session::csrf(session::cookie(cookie.as_bytes()).unwrap());
    let invalid = format!("_csrf={csrf}&action=bands-11,0,0,0,0");
    status(&exchange(&web, "POST", "/tuning", &headers, &invalid).await, 400);
    status(&exchange(&web, "POST", "/sound", &headers, "action=up").await, 403);

    let controller = UnixListener::bind(&web.controller).unwrap();
    let mut calls = tokio::spawn(async move {
        for (action, answer) in [
            ("read", "volume=9/31 mute=off"),
            ("up", "volume=10/31 mute=off"),
            ("read-tuning", "eq=5 clear=0 wide=1 bands=2,1,0,-1,-2"),
            ("bands-10,1,0,-1,-10", "eq=5 clear=0 wide=1 bands=10,1,0,-1,-10"),
            ("clear-on", "error Sony reported a different sound setting"),
            ("read", "volume=10/31 mute=off"),
            ("mute", "volume=10/31 mute=on"),
        ] {
            let (stream, _) = controller.accept().await.unwrap();
            let mut peer = BufReader::new(stream);
            let mut command = String::new();
            peer.read_line(&mut command).await.unwrap();
            let fields: Vec<_> = command.split_whitespace().collect();
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[0], "sound");
            assert_eq!(fields[2], action);
            peer.get_mut().write_all(format!("audio off\nsound {} {answer}\n", fields[1]).as_bytes()).await.unwrap();
        }
    });
    let response = exchange(&web, "GET", "/sound", &headers, "").await;
    status(&response, 200);
    assert!(response.contains("9/31"));
    status(&exchange(&web, "POST", "/sound", &headers, &format!("_csrf={csrf}&action=up")).await, 303);
    let response = exchange(&web, "GET", "/tuning", &headers, "").await;
    status(&response, 200);
    assert!(response.contains("330 Hz"));
    assert!(response.contains("value=\"eq-5\" selected"));
    let bands = format!("_csrf={csrf}&band-1=10&band-2=1&band-3=0&band-4=-1&band-5=-10");
    status(&exchange(&web, "POST", "/tuning", &headers, &bands).await, 303);
    status(&exchange(&web, "POST", "/tuning", &headers, &format!("_csrf={csrf}&action=clear-on")).await, 409);
    let response = exchange(&web, "GET", "/sound?live=1", &headers, "").await;
    status(&response, 200);
    assert!(response.contains("Content-Type: text/plain; charset=utf-8"));
    assert!(response.ends_with("\r\n\r\nvolume=10/31 mute=off"));
    let response = exchange(&web, "POST", "/sound?live=1", &headers, &format!("_csrf={csrf}&action=mute")).await;
    status(&response, 200);
    assert!(response.ends_with("\r\n\r\nvolume=10/31 mute=on"));
    tokio::time::timeout(Duration::from_secs(2), &mut calls).await.unwrap().unwrap();
}
