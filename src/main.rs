use ai_gw_lite::config::AppConfig;
use ai_gw_lite::install;
use ai_gw_lite::observability;
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

    match parse_command(&args)? {
        CliCommand::Install => {
            let report = tokio::task::spawn_blocking(install::run_install)
                .await
                .map_err(|err| format!("install task failed: {err}"))?
                .map_err(|err| err.to_string())?;
            println!("install completed.");
            println!("service: {}", report.service_path);
            println!("config: {}", report.config_path);
            if report.config_created {
                println!("default config created at {}", report.config_path);
            } else {
                println!("existing config kept at {}", report.config_path);
            }
            println!("next: sudo systemctl start ai-gw-lite");
            Ok(())
        }
        CliCommand::Run { config_path } => {
            let config = AppConfig::load_from_file(&config_path)
                .map_err(|err| format!("failed to load config `{config_path}`: {err}"))?;
            observability::init_tracing(config.observability.as_ref())?;

            server::run_server(Arc::new(config)).await
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Run { config_path: String },
    Install,
}

fn parse_command(args: &[String]) -> Result<CliCommand, String> {
    match args {
        [flag] if flag == "--install" => Ok(CliCommand::Install),
        [flag, path] if flag == "--config" => Ok(CliCommand::Run {
            config_path: path.clone(),
        }),
        _ => Err(format!("invalid arguments.\n{}", usage_line())),
    }
}

fn print_usage() {
    println!("{}", usage_line());
}

fn usage_line() -> &'static str {
    "Usage:\n  ai-gw-lite --config <path-to-config.yaml>\n  ai-gw-lite --install"
}

#[cfg(test)]
mod tests {
    use super::{CliCommand, parse_command};

    #[test]
    fn parse_run_command() {
        let args = vec!["--config".to_string(), "/tmp/conf.yaml".to_string()];
        assert_eq!(
            parse_command(&args).expect("command should parse"),
            CliCommand::Run {
                config_path: "/tmp/conf.yaml".to_string()
            }
        );
    }

    #[test]
    fn parse_install_command() {
        let args = vec!["--install".to_string()];
        assert_eq!(
            parse_command(&args).expect("command should parse"),
            CliCommand::Install
        );
    }

    #[test]
    fn reject_mixed_command() {
        let args = vec![
            "--install".to_string(),
            "--config".to_string(),
            "/tmp/conf.yaml".to_string(),
        ];
        let err = parse_command(&args).expect_err("command should be rejected");
        assert!(err.contains("invalid arguments"));
    }
}
