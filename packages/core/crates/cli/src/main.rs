//! `vorn`: see the crate docs in `lib.rs`.

use vorn_cli::output::StdIo;

fn main() {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("vorn: could not start: {err}");
            std::process::exit(1);
        }
    };
    let tty = {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    };
    let code = match runtime.block_on(vorn_cli::plan(&argv, &mut StdIo, tty)) {
        vorn_cli::attach::Outcome::Exit(code) => code,
        vorn_cli::attach::Outcome::Tui(tui) => {
            drop(runtime);
            vorn_cli::attach::open(&tui)
        }
    };
    // Exits rather than dropping the runtime: a read of stdin `vorn mcp` left
    // pending runs on a thread the runtime would otherwise wait for.
    std::process::exit(code.code());
}
