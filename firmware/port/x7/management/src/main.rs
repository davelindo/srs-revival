#[cfg(any(target_os = "linux", test))]
mod controller;
mod power;
mod sound;
mod ssh;
mod terminal;
mod web;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[cfg(target_os = "linux")]
fn sound(action: &str) -> Result<(), Error> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    if sound::Action::parse(action).is_none() {
        return Err("unknown sound action".into());
    }
    let mut stream = UnixStream::connect("/run/zd1link")?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(6)))?;
    let token = std::process::id();
    writeln!(stream, "sound {token} {action}")?;
    let prefix = format!("sound {token} ");
    for line in BufReader::new(stream).lines() {
        if let Some(report) = line?.strip_prefix(&prefix) {
            if let Some(error) = report.strip_prefix("error ") {
                return Err(error.to_owned().into());
            }
            println!("{report}");
            return Ok(());
        }
    }
    Err("controller link closed before sound readback".into())
}

fn main() -> Result<(), Error> {
    #[cfg(all(target_os = "linux", target_arch = "arm"))]
    zd1_compat::init();
    let args: Vec<_> = std::env::args().skip(1).collect();
    #[cfg(target_os = "linux")]
    if let [command, action] = args.as_slice()
        && command == "sound"
    {
        return sound(action);
    }
    #[cfg(target_os = "linux")]
    if let [command, image, device] = args.as_slice()
        && command == "driver"
    {
        return controller::load_driver(image, device);
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    #[cfg(target_os = "linux")]
    if let [command, device, settings] = args.as_slice()
        && command == "linkd"
    {
        return runtime.block_on(controller::serve(device, settings));
    }
    let [command, listen, state, keys] = args.as_slice() else {
        return Err("usage: x7-management ssh|web LISTEN STATE_DIR AUTHORIZED_KEYS|SETTINGS_FILE; linkd DEVICE SETTINGS_FILE; driver MODULE DEVICE".into());
    };
    match command.as_str() {
        "ssh" => runtime.block_on(ssh::serve(listen, state, keys)),
        "web" => runtime.block_on(web::serve(listen, state, keys)),
        _ => Err("unknown management service".into()),
    }
}
