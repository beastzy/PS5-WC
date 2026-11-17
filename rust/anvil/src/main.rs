static POSSIBLE_BACKENDS: &[&str] = &[
    #[cfg(feature = "winit")]
    "--winit : Run PS5-WC as a X11 or Wayland client using winit.",
    #[cfg(feature = "udev")]
    "--tty-udev : Run PS5-WC as a tty udev client (requires root if without logind).",
    #[cfg(feature = "x11")]
    "--x11 : Run PS5-WC as an X11 client.",
];

#[cfg(feature = "profile-with-tracy-mem")]
#[global_allocator]
static GLOBAL: profiling::tracy_client::ProfiledAllocator<std::alloc::System> =
    profiling::tracy_client::ProfiledAllocator::new(std::alloc::System, 10);

fn main() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt()
            .compact()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt().compact().init();
    }

    #[cfg(feature = "profile-with-tracy")]
    profiling::tracy_client::Client::start();

    profiling::register_thread!("Main Thread");

    #[cfg(feature = "profile-with-puffin")]
    let _server = puffin_http::Server::new(&format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT)).unwrap();
    #[cfg(feature = "profile-with-puffin")]
    profiling::puffin::set_scopes_on(true);

    let mut args = std::env::args_os().skip(1);
    let arg = args.next().and_then(|arg| arg.into_string().ok());
    if arg.as_deref() == Some("--render-home") {
        let Some(file) = args.next() else {
            eprintln!("Usage: ps5-wc --render-home PATH [welcome|games|apps|settings|control]");
            std::process::exit(2);
        };
        let view = args.next();
        let view = view.as_deref().and_then(|name| name.to_str()).unwrap_or("games");
        if let Err(error) = anvil::home::Home::new().save_preview(std::path::Path::new(&file), view) {
            eprintln!("Failed to render Home: {error}");
            std::process::exit(1);
        }
        return;
    }
    if arg.as_deref() == Some("--version") {
        println!("PS5-WC (Smithay 0.7.0)");
        return;
    }
    match arg.as_ref().map(|s| &s[..]) {
        #[cfg(feature = "winit")]
        Some("--winit") => {
            tracing::info!("Starting PS5-WC with winit backend");
            anvil::winit::run_winit();
        }
        #[cfg(feature = "udev")]
        Some("--tty-udev") => {
            tracing::info!("Starting PS5-WC on a tty using udev");
            anvil::udev::run_udev();
        }
        #[cfg(feature = "x11")]
        Some("--x11") => {
            tracing::info!("Starting PS5-WC with x11 backend");
            anvil::x11::run_x11();
        }
        Some(other) => {
            tracing::error!("Unknown backend: {}", other);
            std::process::exit(2);
        }
        None => {
            #[allow(clippy::disallowed_macros)]
            {
                println!("USAGE: ps5-wc --backend");
                println!();
                println!("Possible backends are:");
                for b in POSSIBLE_BACKENDS {
                    println!("\t{}", b);
                }
            }
        }
    }
}
