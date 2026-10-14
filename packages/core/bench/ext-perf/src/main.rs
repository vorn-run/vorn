//! `vorn-ext-perf`: Tasks and Workflows built in against the same work as extensions, on test vornds, interleaved run by run.

#[path = "../../../crates/vorn-app/tests/support/mod.rs"]
mod support;

mod board;
mod flows;
mod kit;
mod live;
mod micro;
mod report;
mod run;
mod stats;
mod sys;
mod tasks;

const USAGE: &str = "\
Usage: vorn-ext-perf run --wasm DIR --bin DIR --out DIR [--reps 5] [--only KINDS] [--tiers 100,1000,10000]
       vorn-ext-perf report DIR
       vorn-ext-perf case KIND ARGS...   (one measurement, as JSON; run starts these)

KINDS: micro, tasks, flows, gates, schedule, board.
--wasm holds ext_perf_tasks.wasm and ext_perf_workflows.wasm; --bin holds vornd and vorn-sessiond.";

/// Which side of the comparison a case measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Today's code in vornd and the app.
    Builtin,
    /// The component, changes told to viewers as rows.
    Ext,
    /// The component, changes told as today: the whole configuration.
    ExtConfig,
    /// The step loop's own code compiled into vornd: the design without the sandbox.
    Native,
    /// The kit's board on the component's rows, every card built.
    Kit,
    /// The kit's board drawing only the cards a column shows.
    KitWindow,
}

impl Variant {
    pub fn parse(s: &str) -> Result<Variant, String> {
        Ok(match s {
            "builtin" => Variant::Builtin,
            "ext" => Variant::Ext,
            "ext-config" => Variant::ExtConfig,
            "native" => Variant::Native,
            "kit" => Variant::Kit,
            "kit-window" => Variant::KitWindow,
            _ => return Err(format!("unknown variant {s:?}")),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Variant::Builtin => "builtin",
            Variant::Ext => "ext",
            Variant::ExtConfig => "ext-config",
            Variant::Native => "native",
            Variant::Kit => "kit",
            Variant::KitWindow => "kit-window",
        }
    }

    /// The environment a test vornd starts with to take `feature` over this way.
    pub fn apply(self, feature: &str) {
        for k in [
            "VORN_EXT_SPIKE",
            "VORN_EXT_SPIKE_RUN",
            "VORN_EXT_SPIKE_ANNOUNCE",
        ] {
            std::env::remove_var(k);
        }
        match self {
            Variant::Builtin | Variant::Kit | Variant::KitWindow => {}
            Variant::Ext => std::env::set_var("VORN_EXT_SPIKE", feature),
            Variant::ExtConfig => {
                std::env::set_var("VORN_EXT_SPIKE", feature);
                std::env::set_var("VORN_EXT_SPIKE_ANNOUNCE", "config");
            }
            Variant::Native => {
                std::env::set_var("VORN_EXT_SPIKE", feature);
                std::env::set_var("VORN_EXT_SPIKE_RUN", "native");
            }
        }
    }
}

fn num(args: &[String], i: usize) -> Result<usize, String> {
    args.get(i)
        .ok_or(USAGE)?
        .parse()
        .map_err(|_| format!("bad number {:?}", args[i]))
}

fn case(args: &[String]) -> Result<serde_json::Value, String> {
    let kind = args.first().map(String::as_str).ok_or(USAGE)?;
    let variant = || Variant::parse(args.get(1).map_or("", String::as_str));
    let runtime = || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    };
    match kind {
        "micro" => micro::case(
            num(args, 1)?,
            std::path::Path::new(args.get(2).ok_or(USAGE)?),
        ),
        "tasks" => runtime()?.block_on(tasks::case(variant()?, num(args, 2)?)),
        "flows" => runtime()?.block_on(flows::runs(variant()?, num(args, 2)?, num(args, 3)?)),
        "gates" => runtime()?.block_on(flows::gates(variant()?, num(args, 2)?)),
        "schedule" => runtime()?.block_on(flows::schedule(variant()?)),
        "board" => board::case(variant()?, num(args, 2)?),
        _ => Err(format!("unknown case {kind:?}\n{USAGE}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("case") => case(&args[1..]).map(|v| println!("{v}")),
        Some("run") => run::run(&args[1..]),
        Some("report") => args
            .get(1)
            .ok_or_else(|| USAGE.to_owned())
            .and_then(|d| report::write(std::path::Path::new(d))),
        _ => Err(USAGE.to_owned()),
    };
    if let Err(e) = result {
        eprintln!("vorn-ext-perf: {e}");
        std::process::exit(1);
    }
}
