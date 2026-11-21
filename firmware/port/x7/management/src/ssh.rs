//! Authenticated SSH command execution. The transport and cryptography are
//! supplied by russh; no SSH wire format or cryptographic primitive is ours.
use crate::{Error, terminal};
use russh::{
    Channel, ChannelId, ChannelMsg, Preferred, cipher, kex,
    keys::{self, Algorithm, PrivateKey, PublicKey},
    server::{self, Msg, Session, VerifiedPublicKey},
};
use std::{borrow::Cow, fs, io::Read, path::Path, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
    sync::{Semaphore, oneshot},
    task::JoinHandle,
};

fn host_key(state: &Path) -> Result<PrivateKey, Error> {
    fs::create_dir_all(state)?;
    let path = state.join("ssh_host_ed25519_key");
    if path.exists() {
        return Ok(keys::load_secret_key(&path, None)?);
    }
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
    let encoded = key.to_openssh(keys::ssh_key::LineEnding::LF)?;
    zd1_sys::write_private(path.to_str().ok_or("invalid host-key path")?, &encoded)?;
    Ok(key)
}

fn authorized(path: &str) -> Result<Vec<PublicKey>, Error> {
    let mut text = String::new();
    fs::File::open(path)?.take(8193).read_to_string(&mut text)?;
    if text.len() > 8192 {
        return Err("authorized_keys exceeds 8 KiB".into());
    }
    let mut keys = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let key = PublicKey::from_openssh(line)?;
        if key.algorithm() != Algorithm::Ed25519 {
            return Err("only Ed25519 login keys are enabled".into());
        }
        if keys.len() == 16 {
            return Err("at most 16 login keys are allowed".into());
        }
        keys.push(key);
    }
    if keys.is_empty() {
        return Err("no SSH login key provisioned".into());
    }
    Ok(keys)
}

pub async fn serve(listen: &str, state: &str, key_file: &str) -> Result<(), Error> {
    let authorized = Arc::new(authorized(key_file)?);
    let config = Arc::new(server::Config {
        keys: vec![host_key(Path::new(state))?],
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::ZERO),
        inactivity_timeout: Some(Duration::from_secs(3600)),
        window_size: 32 * 1024,
        maximum_packet_size: 4096,
        channel_buffer_size: 8,
        event_buffer_size: 8,
        max_auth_attempts: 4,
        preferred: Preferred {
            kex: Cow::Borrowed(&[kex::CURVE25519]),
            key: Cow::Borrowed(&[Algorithm::Ed25519]),
            cipher: Cow::Borrowed(&[cipher::CHACHA20_POLY1305]),
            ..Preferred::default()
        },
        ..server::Config::default()
    });
    let listener = TcpListener::bind(listen).await?;
    let limit = Arc::new(Semaphore::new(2));
    eprintln!("SSH listening on {}", listener.local_addr()?);
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(permit) = limit.clone().try_acquire_owned() else {
            continue;
        };
        let config = config.clone();
        let keys = authorized.clone();
        let socket = stream.into_std()?;
        let close = socket.try_clone()?;
        let stream = tokio::net::TcpStream::from_std(socket)?;
        tokio::spawn(async move {
            let _permit = permit;
            let (authenticated, mut authentication) = oneshot::channel();
            let handler = Handler {
                keys,
                authenticated: Some(authenticated),
                channel: None,
                command: None,
                active: None,
                pty: None,
                terminal: None,
            };
            // Include the banner/key exchange in the same absolute deadline.
            // Inactivity timeouts alone let an unauthenticated peer occupy a
            // scarce connection slot indefinitely by trickling packets.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            match tokio::time::timeout_at(deadline, server::run_stream(config, stream, handler)).await {
                Ok(Ok(mut session)) => {
                    tokio::select! {
                        _ = &mut session => {},
                        _ = &mut authentication => { let _ = session.await; },
                        _ = tokio::time::sleep_until(deadline) => {
                            let _ = close.shutdown(std::net::Shutdown::Both);
                            let _ = session.await;
                        }
                    }
                }
                Ok(Err(error)) => eprintln!("SSH session: {error}"),
                Err(_) => {
                    let _ = close.shutdown(std::net::Shutdown::Both);
                }
            }
        });
    }
}

struct Handler {
    keys: Arc<Vec<PublicKey>>,
    authenticated: Option<oneshot::Sender<()>>,
    channel: Option<Channel<Msg>>,
    command: Option<JoinHandle<()>>,
    active: Option<ChannelId>,
    pty: Option<terminal::Pending>,
    terminal: Option<Arc<terminal::Master>>,
}

impl Drop for Handler {
    fn drop(&mut self) {
        if let Some(task) = self.command.take() {
            task.abort();
        }
    }
}

impl Handler {
    fn launch(&mut self, id: ChannelId, command: Option<String>, session: &mut Session) -> Result<(), Error> {
        if self.channel.as_ref().is_none_or(|channel| channel.id() != id) {
            session.channel_failure(id)?;
            return Ok(());
        }
        let channel = self.channel.take().ok_or("channel disappeared")?;
        let handle = session.handle();
        let pty = self.pty.take();
        session.channel_success(id)?;
        self.command = Some(tokio::spawn(async move {
            let result = match pty {
                Some(pty) => execute_terminal(command, channel, handle.clone(), pty).await,
                None => execute(command, channel, handle.clone()).await,
            };
            if let Err(error) = result {
                eprintln!("SSH command: {error}");
                let _ = handle.exit_status_request(id, 1).await;
                let _ = handle.close(id).await;
            }
        }));
        Ok(())
    }
}

impl server::Handler for Handler {
    type Error = Error;

    async fn auth_publickey(&mut self, user: &str, key: VerifiedPublicKey<'_>) -> Result<server::Auth, Error> {
        Ok(if user == "root" && self.keys.iter().any(|allowed| allowed.key_data() == key.public_key.key_data()) {
            if let Some(authenticated) = self.authenticated.take() {
                let _ = authenticated.send(());
            }
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Error> {
        if self.channel.is_some() || self.command.as_ref().is_some_and(|task| !task.is_finished()) {
            return Ok(());
        }
        self.command.take();
        self.terminal.take();
        self.pty.take();
        self.active = Some(channel.id());
        self.channel = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(&mut self, id: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Error> {
        let Ok(command) = std::str::from_utf8(data) else {
            session.channel_failure(id)?;
            return Ok(());
        };
        if command.len() > 4096 || self.channel.as_ref().is_none_or(|channel| channel.id() != id) {
            session.channel_failure(id)?;
            return Ok(());
        }
        self.launch(id, Some(command.to_owned()), session)
    }

    async fn shell_request(&mut self, id: ChannelId, session: &mut Session) -> Result<(), Error> {
        self.launch(id, None, session)
    }

    async fn pty_request(
        &mut self,
        id: ChannelId,
        term: &str,
        columns: u32,
        rows: u32,
        _pixel_width: u32,
        _pixel_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Error> {
        if self.channel.as_ref().is_none_or(|channel| channel.id() != id) || self.pty.is_some() {
            session.channel_failure(id)?;
            return Ok(());
        }
        match terminal::Pending::open(term, columns, rows) {
            Ok(pty) => {
                self.terminal = Some(pty.master.clone());
                self.pty = Some(pty);
                session.channel_success(id)?;
            }
            Err(_) => session.channel_failure(id)?,
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        id: ChannelId,
        columns: u32,
        rows: u32,
        _pixel_width: u32,
        _pixel_height: u32,
        _session: &mut Session,
    ) -> Result<(), Error> {
        if self.active == Some(id)
            && let Some(terminal) = &self.terminal
        {
            terminal.resize(columns, rows)?;
        }
        Ok(())
    }

    async fn channel_close(&mut self, id: ChannelId, _session: &mut Session) -> Result<(), Error> {
        if self.active == Some(id) {
            if let Some(task) = self.command.take() {
                task.abort();
            }
            self.terminal.take();
            self.pty.take();
            self.channel.take();
            self.active = None;
        }
        Ok(())
    }
}

async fn output(
    mut reader: impl AsyncRead + Unpin,
    id: ChannelId,
    handle: server::Handle,
    stderr: bool,
) -> Result<(), Error> {
    let mut buffer = [0; 2048];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            return Ok(());
        }
        let bytes = buffer[..n].to_vec();
        if stderr {
            handle.extended_data(id, 1, bytes).await.map_err(|_| "SSH output closed")?;
        } else {
            handle.data(id, bytes).await.map_err(|_| "SSH output closed")?;
        }
    }
}

fn program(command: Option<String>, interactive: bool) -> Command {
    let mut program = Command::new("/bin/sh");
    if let Some(command) = command {
        program.arg("-c").arg(command);
    } else {
        program.arg(if interactive { "-i" } else { "-s" });
    }
    program.kill_on_drop(true);
    program
}

async fn finish(id: ChannelId, handle: &server::Handle, status: std::process::ExitStatus) -> Result<(), Error> {
    handle.exit_status_request(id, status.code().unwrap_or(1) as u32).await.map_err(|()| "SSH session closed")?;
    handle.eof(id).await.map_err(|()| "SSH session closed")?;
    handle.close(id).await.map_err(|()| "SSH session closed")?;
    Ok(())
}

async fn execute(command: Option<String>, mut channel: Channel<Msg>, handle: server::Handle) -> Result<(), Error> {
    let id = channel.id();
    let mut program = program(command, false);
    unsafe {
        program.pre_exec(|| terminal::session(false));
    }
    let mut child = program
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    drop(program);
    let _group = terminal::ProcessGroup(child.id().ok_or("missing child PID")? as i32);
    let mut stdin = child.stdin.take().ok_or("missing command stdin")?;
    let stdout = child.stdout.take().ok_or("missing command stdout")?;
    let stderr = child.stderr.take().ok_or("missing command stderr")?;
    let input = async move {
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } if stdin.write_all(&data).await.is_err() => break,
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
    };
    // One owned future contains every pipe operation. Cancelling it drops the
    // child and its IO immediately, without detached per-pipe worker tasks.
    let completion = async {
        let (status, _, _) = tokio::try_join!(
            async { Ok::<_, Error>(child.wait().await?) },
            output(stdout, id, handle.clone(), false),
            output(stderr, id, handle.clone(), true)
        )?;
        Ok::<_, Error>(status)
    };
    tokio::pin!(completion);
    let status = tokio::select! { result = &mut completion => result?, () = input => completion.await? };
    finish(id, &handle, status).await
}

async fn execute_terminal(
    command: Option<String>,
    mut channel: Channel<Msg>,
    handle: server::Handle,
    pty: terminal::Pending,
) -> Result<(), Error> {
    let id = channel.id();
    let terminal::Pending { master, slave, term } = pty;
    let mut program = program(command, true);
    program.env("TERM", term).stdin(slave.try_clone()?).stdout(slave.try_clone()?).stderr(slave.try_clone()?);
    unsafe {
        program.pre_exec(|| terminal::session(true));
    }
    let mut child = program.spawn()?;
    // Command retains Stdio::Owned handles so it can be spawned again. Those
    // parent-side slave copies must close before waiting for PTY EOF on Linux.
    drop(program);
    let _group = terminal::ProcessGroup(child.id().ok_or("missing child PID")? as i32);
    drop(slave); // only the child may retain a slave, so its exit produces EOF
    let input = async {
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } if master.write_all(&data).await.is_err() => break,
                ChannelMsg::Eof => {
                    let _ = master.write_all(&[4]).await;
                    break;
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
    };
    let output = async {
        let mut bytes = [0; 2048];
        loop {
            let n = master.read(&mut bytes).await?;
            if n == 0 {
                return Ok::<_, Error>(());
            }
            handle.data(id, bytes[..n].to_vec()).await.map_err(|_| "SSH output closed")?;
        }
    };
    let completion = async {
        let (status, _) = tokio::try_join!(async { Ok::<_, Error>(child.wait().await?) }, output)?;
        Ok::<_, Error>(status)
    };
    tokio::pin!(completion);
    let status = tokio::select! { result = &mut completion => result?, () = input => completion.await? };
    finish(id, &handle, status).await
}
