use ai_gw_lite::config::AppConfig;
use ai_gw_lite::server;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return Ok(());
    }

    let config_path = parse_config_path(&args)?;
    let config = AppConfig::load_from_file(&config_path)
        .map_err(|err| format!("failed to load config `{config_path}`: {err}"))?;

    server::run_server(Arc::new(config)).await
}

fn parse_config_path(args: &[String]) -> Result<String, String> {
    match args {
        [flag, path] if flag == "--config" => Ok(path.clone()),
        _ => Err(format!("invalid arguments.\n{}", usage_line())),
    }
}

fn print_usage() {
    println!("{}", usage_line());
}

fn usage_line() -> &'static str {
    "Usage: ai-gw-lite --config <path-to-config.yaml>"
}
