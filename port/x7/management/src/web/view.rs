use super::Web;
use crate::sound::{BANDS, Preset, parse_bands};
use maud::{Markup, PreEscaped, html};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const STYLE: &str = include_str!("dashboard.css");

fn field(web: &Web, key: &str, label: &str, hint: &str) -> Markup {
    html! { label for=(key) { (label) } input id=(key) name=(key) value=(web.get(key)); @if !hint.is_empty() { small { (hint) } } }
}

fn mode(web: &Web, key: &str, label: &str) -> Markup {
    let value = web.get(key);
    html! { label for=(key) { (label) } select id=(key) name=(key) {
        @for (option, title) in [("dhcp", "Automatic (DHCP)"), ("static", "Fixed address"), ("off", "Off")] {
            option value=(option) selected[value == option] { (title) }
        }
    } }
}

fn hidden(csrf: &str, destination: &str) -> Markup {
    html! { input type="hidden" name="_csrf" value=(csrf); input type="hidden" name="_return" value=(destination); }
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| "Not available".into())
}

fn tail(path: &str) -> String {
    let result = (|| -> std::io::Result<String> {
        let mut file = fs::File::open(path)?;
        let len = file.metadata()?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(16384)))?;
        let mut bytes = Vec::new();
        file.take(16384).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })();
    result.unwrap_or_else(|_| "No log available.".into())
}

fn memory() -> (u64, u64) {
    let text = read("/proc/meminfo");
    let value = |key| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    (value("MemTotal:"), value("MemFree:"))
}

fn document(title: &str, body: Markup) -> String {
    html! {
        (maud::DOCTYPE) html lang="en" {
            head { meta charset="utf-8"; meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " · SRS-X7" } style { (PreEscaped(STYLE)) } }
            body { (body) }
        }
    }
    .into_string()
}

pub(super) fn login(web: &Web, error: Option<&str>) -> String {
    document(
        "Sign in",
        html! { main.shell { section.card.signin {
            p.brand { "Sony SRS-X7" } h1 { "Sign in" }
            @if let Some(error) = error { div.notice role="alert" { (error) } }
            form method="post" action="/login" {
                input type="hidden" name="_csrf" value=(web.login_csrf);
                label for="login-password" { "Speaker password" }
                input id="login-password" name="password" type="password" required maxlength="128" autocomplete="current-password";
                button type="submit" { "Sign in" }
            }
        }}},
    )
}

pub(super) fn page(web: &Web, csrf: &str, path: &str, notice: Option<&str>, detail: Option<&str>) -> String {
    let name = web.get("name");
    let sound_page = matches!(path, "/" | "/sound" | "/tuning");
    let title = match path {
        "/network" => "Network",
        "/system" => "System",
        "/power" => "Sleep & standby",
        "/ssh" => "SSH access",
        "/debug" => "Diagnostics",
        _ => "Sound & controls",
    };
    let body = match path {
        "/network" => network(web, csrf),
        "/power" => power(web, csrf),
        "/ssh" => ssh(web, csrf),
        "/debug" => debug(detail.unwrap_or("Service status unavailable.")),
        "/system" => overview(web, csrf),
        _ => sound(web, csrf, path, detail),
    };
    document(
        title,
        html! {
            header { div.shell {
                a.brand href="/" { span.brand-mark aria-hidden="true" { i {} i {} i {} i {} } span { (name) @if name != "SRS-X7" { small { "Sony SRS-X7" } } } }
                nav aria-label="Speaker settings" {
                    @for (target, label) in [("/", "Sound"), ("/network", "Network"), ("/power", "Standby"), ("/system", "System"), ("/ssh", "SSH"), ("/debug", "Diagnostics")] {
                        a href=(target) aria-current=[(path == target || (sound_page && target == "/")).then_some("page")] { (label) }
                    }
                }
                form method="post" action="/logout" { (hidden(csrf, "/")) button.subtle type="submit" { "Sign out" } }
            }}
            main.shell {
                @if !sound_page { h1 { (title) } }
                @if web.recovery() { div.notice role="status" { "Recovery mode. Connect over Ethernet and use SSH to repair or boot a system slot." } }
                @if let Some(notice) = notice { div.notice role="status" { (notice) } }
                (body)
            }
            @if sound_page && !web.recovery() { script src="/dashboard.js" defer {} }
        },
    )
}

fn overview(web: &Web, csrf: &str) -> Markup {
    let (total, free) = memory();
    let load = read("/proc/loadavg");
    let load = load.split_whitespace().next().filter(|value| value.parse::<f64>().is_ok()).unwrap_or("Unavailable");
    html! {
        div.grid {
            section.card { h2 { "Speaker" }
                form method="post" action="/settings" { (hidden(csrf, "/system"))
                    label for="name" { "Speaker name" } input id="name" name="name" value=(web.get("name")) maxlength="63";
                    button type="submit" { "Save speaker name" }
                }
                a.button href="/sound" { "Open sound controls" }
            }
            section.card { h2 { "System" }
                dl {
                    @if total != 0 {
                        dt { "Free RAM" } dd { (free / 1024) " MiB" }
                        dt { "Usable RAM" } dd { (total / 1024) " MiB" }
                    }
                    dt { "Load average" } dd { (load) }
                }
                @if total != 0 { meter aria-label="RAM in use" min="0" max=(total) value=(total.saturating_sub(free)) { (total.saturating_sub(free)) } }
                a.button href="/debug" { "Open diagnostics" }
            }
        }
    }
}

fn report_value<'a>(report: Option<&'a str>, key: &str) -> Option<&'a str> {
    report?.split_whitespace().find_map(|word| word.strip_prefix(key))
}

fn sound(web: &Web, csrf: &str, path: &str, readback: Option<&str>) -> Markup {
    let tuning = readback.filter(|line| line.starts_with("eq="));
    let volume = readback.filter(|line| line.starts_with("volume="));
    let recovery = web.recovery();
    let scope = if path == "/tuning" { "tuning" } else { "sound" };
    html! {
        div id="sound-dashboard" data-scope=(scope) data-sound=[volume] data-tuning=[tuning] {
            div.page-heading {
                h1 { "Sound" }
                div.sync {
                    span id="controller-status" role="status" { (if recovery { "Recovery mode" } else { "Controller" }) }
                    button.subtle id="refresh-audio" type="button" hidden { "Refresh" }
                }
            }
            p.notice id="audio-error" role="alert" hidden {}
            div.mixer {
                (equalizer(csrf, tuning))
                div.mixer-side {
                    section.card.output {
                        h2 { "Master volume" }
                        @let reading = report_value(volume, "volume=").and_then(|text| text.split_once('/'));
                        div.volume-reading { output id="volume-level" { (reading.map_or("—", |value| value.0)) } span id="volume-maximum" { (reading.map_or("", |value| value.1)) } }
                        meter id="volume-meter" aria-label="Master volume" min="0" max="31" value=(reading.and_then(|value| value.0.parse::<u8>().ok()).unwrap_or(0)) {}
                        form method="post" action="/sound" class="transport" data-live="sound" {
                            (hidden(csrf, "/sound"))
                            button.subtle type="submit" name="action" value="down" aria-label="Volume down" disabled[recovery] { "−" }
                            button.subtle type="submit" name="action" value="up" aria-label="Volume up" disabled[recovery] { "+" }
                            button.subtle type="submit" name="action" value="mute" id="mute-control" disabled[recovery] { "Mute" }
                            button.subtle.fallback type="submit" name="action" value="unmute" disabled[recovery] { "Unmute" }
                        }
                        noscript { a href="/sound" { "Read volume" } }
                    }
                    section.card.processing {
                        h2 { "Sound modes" }
                        (mode_control(csrf, tuning, "clear", "ClearAudio+", [("clear-off", "Off"), ("clear-on", "On")]))
                        (mode_control(csrf, tuning, "wide", "Wide Stereo", [("wide-normal", "Normal"), ("wide-high", "High")]))
                    }
                }
            }
            (receivers(web, csrf))
        }
    }
}

fn mode_control(csrf: &str, report: Option<&str>, id: &str, label: &str, options: [(&str, &str); 2]) -> Markup {
    let enabled = report_value(report, &format!("{id}=")) == Some("1");
    html! {
        form method="post" action="/tuning" class="mode-row" data-live="tuning" {
            (hidden(csrf, "/tuning"))
            label for=(id) { (label) }
            select id=(id) name="action" data-commit disabled[report.is_none()] {
                @for (index, (value, title)) in options.into_iter().enumerate() {
                    option value=(value) selected[index == usize::from(enabled)] { (title) }
                }
            }
            button.subtle.fallback type="submit" disabled[report.is_none()] { "Apply" }
        }
    }
}

fn equalizer(csrf: &str, report: Option<&str>) -> Markup {
    let current = report_value(report, "eq=").and_then(|code| code.parse::<usize>().ok());
    let bands = report_value(report, "bands=").and_then(parse_bands).unwrap_or([0; 5]);
    html! {
        section.card.equalizer {
            div.panel-heading {
                div { h2 { "Equalizer" } p.muted id="curve-caption" { "Custom curve · steps" } }
                form.preset method="post" action="/tuning" data-live="tuning" { (hidden(csrf, "/tuning"))
                    label for="eq" { "Equalizer preset" }
                    select id="eq" name="action" data-commit disabled[current.is_none()] {
                        option value="" disabled selected[current.is_none()] { "Loading…" }
                        @for preset in Preset::ALL {
                            option value=(format!("eq-{}", preset.code())) selected[current == Some(preset.code() as usize)] { (preset.name()) }
                        }
                    }
                    button.subtle.fallback type="submit" disabled[current.is_none()] { "Apply preset" }
                }
            }
            div.chart id="eq-chart" hidden {
                canvas id="eq-curve" aria-hidden="true" {}
                div.chart-scale aria-hidden="true" { span { "+10" } span { "0" } span { "−10" } }
                @for (index, frequency) in BANDS.into_iter().enumerate() {
                    div.chart-point data-point=(index) aria-hidden="true" title=(frequency) {}
                }
            }
            form method="post" action="/tuning" id="eq-bands" data-live="tuning" {
                (hidden(csrf, "/tuning"))
                div.bands {
                    @for (index, (frequency, level)) in BANDS.into_iter().zip(bands).enumerate() {
                        @let key = format!("band-{}", index + 1);
                        div.band {
                            label for=(&key) { (frequency) }
                            output for=(&key) data-band-value=(index) { (level) }
                            input type="range" id=(&key) name=(key) min="-10" max="10" step="1" value=(level) disabled[current.is_none()];
                        }
                    }
                }
                div.eq-actions {
                    span.muted id="eq-state" role="status" { "Apply the five band levels as Custom EQ." }
                    div.button-row {
                        button.subtle id="reset-bands" type="button" hidden { "Zero bands" }
                        button type="submit" id="apply-eq" disabled[current.is_none()] { "Apply custom EQ" }
                    }
                }
            }
            noscript { a href="/tuning" { "Read equalizer settings" } }
        }
    }
}

fn receivers(web: &Web, csrf: &str) -> Markup {
    let bitrate = web.get("spotify.bitrate");
    html! {
        details.card.receivers {
            summary { span { "Streaming settings" } small { "AirPlay 2 · Spotify Connect · Snapcast" } }
            form method="post" action="/settings" { (hidden(csrf, "/sound"))
                div.receiver-grid {
                    @for (key, label) in [("airplay", "AirPlay 2"), ("spotify", "Spotify Connect"), ("snapcast", "Snapcast")] {
                        section {
                            h3 { (label) }
                            label for=(key) { "Receiver" }
                            select id=(key) name=(key) {
                                option value="on" selected[web.get(key) == "on"] { "Enabled" }
                                option value="off" selected[web.get(key) == "off"] { "Disabled" }
                            }
                            @match key {
                                "airplay" => { (field(web, "airplay.pin", "Pairing PIN", "Four digits, or leave blank for transient pairing.")) }
                                "spotify" => {
                                    label for="spotify.bitrate" { "Bitrate" }
                                    select id="spotify.bitrate" name="spotify.bitrate" {
                                        @for option in ["96", "160", "320"] {
                                            option value=(option) selected[bitrate == option] { (option) " kbps" }
                                        }
                                    }
                                }
                                _ => {
                                    (field(web, "snapcast.server", "Snapcast server", "Leave blank for automatic discovery."))
                                    (field(web, "snapcast.latency", "Output delay (ms)", ""))
                                }
                            }
                        }
                    }
                }
                button type="submit" { "Save streaming settings" }
            }
        }
    }
}

fn network(web: &Web, csrf: &str) -> Markup {
    let networks = zd1_settings::networks_in(web.network_dir());
    html! {
        form method="post" action="/settings" { (hidden(csrf, "/network"))
            div.grid { @for (prefix, title) in [("eth", "Ethernet"), ("wifi", "Wi-Fi")] {
                section.card { h2 { (title) }
                    (mode(web, &format!("{prefix}.mode"), "Address mode"))
                    (field(web, &format!("{prefix}.address"), "Fixed address", "Address/prefix, for example 192.0.2.10/24."))
                    (field(web, &format!("{prefix}.gateway"), "Gateway", "Used with a fixed address."))
                    (field(web, &format!("{prefix}.dns"), "DNS server", "Used with a fixed address."))
                }
            } }
            section.card { p.muted { "After changing an address, reconnect using the new address." } button type="submit" { "Apply network settings" } }
        }
        section.card { h2 { "Known Wi-Fi networks" }
            @if networks.is_empty() { p.muted { "No networks saved." } }
            @else { ul { @for network in &networks { li { (network.ssid) " · " (if network.psk.is_some() { "WPA2" } else { "Open" }) } } } }
            form method="post" action="/wifi" { (hidden(csrf, "/network"))
                label for="ssid" { "Network name" } input id="ssid" name="ssid" maxlength="32" required;
                label for="password" { "Wi-Fi passphrase" } input id="password" type="password" name="password" autocomplete="new-password" maxlength="63";
                label { input type="checkbox" name="open"; "This is an open network" }
                button type="submit" { "Add network" }
            }
        }
    }
}

fn ssh(web: &Web, csrf: &str) -> Markup {
    let keys = fs::read_to_string(web.state.join("authorized_keys")).unwrap_or_default();
    html! { section.card { h2 { "SSH access" }
        p { "Connect with " code { "ssh root@<speaker-address>" } ". Use your Ed25519 private key on your computer." }
        p.muted { "Paste public keys below. Password login is disabled." }
        form method="post" action="/keys" { (hidden(csrf, "/ssh"))
            label for="keys" { "Authorized public keys" }
            textarea id="keys" name="keys" spellcheck="false" required { (keys) }
            button type="submit" { "Save SSH keys" }
        }
    } }
}

fn power(web: &Web, csrf: &str) -> Markup {
    let desired = crate::power::load(&web.settings);
    let report = read(crate::power::REPORT);
    let value = |name: &str| report.lines().find_map(|line| line.strip_prefix(name)).unwrap_or("Unknown");
    html! {
        div.grid {
            section.card { h2 { "Sleep & standby" }
                p { "Keep the network available in standby while the AC adaptor is connected. Battery standby and low-battery shutdown remain controlled by Sony." }
                form method="post" action="/power" { (hidden(csrf, "/power"))
                    label for="network" { "Network standby on mains power" }
                    select id="network" name="network" {
                        option value="on" selected[desired.network] { "Keep network available" }
                        option value="off" selected[!desired.network] { "Off" }
                    }
                    label for="automatic" { "Automatic standby when idle" }
                    select id="automatic" name="automatic" {
                        option value="keep" selected[desired.automatic.is_none()] { "Preserve Sony’s setting" }
                        option value="on" selected[desired.automatic == Some(true)] { "Enabled" }
                        option value="off" selected[desired.automatic == Some(false)] { "Disabled" }
                    }
                    button type="submit" { "Apply standby settings" }
                }
            }
            section.card { h2 { "Controller readback" }
                dl {
                    dt { "Status" } dd { (value("status=")) }
                    dt { "Network standby" } dd { (value("network_standby=")) }
                    dt { "Automatic standby" } dd { (value("auto_standby=")) }
                    dt { "Battery" } dd { (value("battery=")) }
                }
                p.muted { "These are the last controller replies. Battery levels are approximate; charging hides capacity. Apply the settings to request a fresh reading." }
            }
        }
    }
}

fn debug(services: &str) -> Markup {
    let image = if Path::new("/.zd1-system").is_file() { "/.zd1-system" } else { "/usr/share/x7/system-id" };
    html! {
        section.card { h2 { "Boot" } h3 { "Kernel" } pre { (read("/proc/sys/kernel/osrelease")) }
            h3 { "System image" } pre { (read(image)) } }
        section.card { h2 { "Services" } pre { (services) } }
        section.card { h2 { "Controller link" } pre { (read("/dev/x7-spi")) } }
        section.card { h2 { "Standby and battery" } pre { (read(crate::power::REPORT)) } }
        div.grid { section.card { h2 { "Memory" } pre { (read("/proc/meminfo")) } }
            section.card { h2 { "Network interfaces" } pre { (read("/proc/net/dev")) } h3 { "Routes" } pre { (read("/proc/net/route")) } }
        }
        @for (title, file) in [("AirPlay", "/var/log/airplay.log"), ("Wi-Fi", "/var/log/wifi.log"), ("Ethernet", "/var/log/ethernet.log"),
            ("Controller", "/var/log/sony-link.log"), ("SPI driver", "/var/log/sony-spi.log"), ("SSH", "/var/log/ssh.log"), ("Web interface", "/var/log/web.log")] {
            @if Path::new(file).exists() { section.card { h2 { (title) " log" } pre { (tail(file)) } } }
        }
        section.card { p.muted { "Logs show the most recent 16 KiB. Reload this page to refresh." } a.button href="/debug" { "Refresh diagnostics" } }
    }
}
