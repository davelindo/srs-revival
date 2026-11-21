fn main() {
    // The catalogue is the Sony table at 0x6051c9a4. Its original
    // initializer (0x60195fe8) uses words 3/4 as the initial fixed bytes.
    println!("cargo:rerun-if-changed=sony-registers.csv");
    let catalogue = std::fs::read_to_string("sony-registers.csv").unwrap();
    let mut fixed = String::from("const FIXED: &[(u8, [u8; 2])] = &[\n");
    for line in catalogue.lines().skip(1) {
        let fields: Vec<u8> = line.split(',').map(|n| n.parse().expect("invalid register byte")).collect();
        let [register, a, b] = fields[..] else { panic!("expected register,a,b") };
        fixed.push_str(&format!("({register}, [{a}, {b}]),\n"));
    }
    fixed.push_str("];\n");
    std::fs::write(std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("sony-fixed.rs"), fixed).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("arm")
    {
        for call in ["syscall", "epoll_create1", "eventfd"] {
            println!("cargo:rustc-link-arg-bins=-Wl,--wrap={call}");
        }
    }
}
