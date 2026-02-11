use std::fmt;

#[cfg(target_os = "linux")]
const LINUX_CONFIG_DIR: &str = "/etc/ai_gw_lite";
#[cfg(target_os = "linux")]
const LINUX_CONFIG_PATH: &str = "/etc/ai_gw_lite/conf.yaml";
#[cfg(target_os = "linux")]
const SYSTEMD_SERVICE_NAME: &str = "ai-gw-lite";
#[cfg(target_os = "linux")]
const SYSTEMD_SERVICE_PATH: &str = "/etc/systemd/system/ai-gw-lite.service";

#[cfg(target_os = "linux")]
const DEFAULT_CONFIG_TEMPLATE: &str = r#"listen: "0.0.0.0:8080"

gateway_auth:
  tokens:
    - "${GW_TOKEN}"
  token_sources:
    - type: "authorization_bearer"

routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      strip_prefix: true
      connect_timeout_ms: 10000
      request_timeout_ms: 60000
      inject_headers:
        - name: "authorization"
          value: "Bearer ${OPENAI_API_KEY}"
      remove_headers:
        - "authorization"
        - "x-forwarded-for"
        - "forwarded"
        - "cf-connecting-ip"
        - "true-client-ip"
      forward_xff: false
"#;

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub config_path: &'static str,
    pub service_path: &'static str,
    pub config_created: bool,
}

#[derive(Debug)]
pub enum InstallError {
    UnsupportedPlatform,
    CurrentExe(std::io::Error),
    CanonicalizeExe {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    CommandIo {
        command: String,
        source: std::io::Error,
    },
    CommandFailed {
        command: String,
        status_code: Option<i32>,
        stderr: String,
    },
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                write!(f, "--install is only supported on Linux")
            }
            Self::CurrentExe(source) => {
                write!(f, "failed to resolve current executable path: {source}")
            }
            Self::CanonicalizeExe { path, source } => {
                write!(
                    f,
                    "failed to canonicalize executable path `{}`: {source}",
                    path.display()
                )
            }
            Self::Io { path, source } => {
                write!(f, "failed to write `{}`: {source}", path.display())
            }
            Self::CommandIo { command, source } => {
                write!(f, "failed to execute command `{command}`: {source}")
            }
            Self::CommandFailed {
                command,
                status_code,
                stderr,
            } => {
                write!(
                    f,
                    "command `{command}` failed with status {:?}: {}",
                    status_code,
                    stderr.trim()
                )
            }
        }
    }
}

impl std::error::Error for InstallError {}

pub fn run_install() -> Result<InstallReport, InstallError> {
    #[cfg(target_os = "linux")]
    {
        run_install_linux()
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(InstallError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
fn run_install_linux() -> Result<InstallReport, InstallError> {
    use std::fs;
    use std::path::{Path, PathBuf};

    let config_dir = Path::new(LINUX_CONFIG_DIR);
    fs::create_dir_all(config_dir).map_err(|source| InstallError::Io {
        path: config_dir.to_path_buf(),
        source,
    })?;

    let config_path = Path::new(LINUX_CONFIG_PATH);
    let config_created = if config_path.exists() {
        false
    } else {
        fs::write(config_path, DEFAULT_CONFIG_TEMPLATE).map_err(|source| InstallError::Io {
            path: config_path.to_path_buf(),
            source,
        })?;
        true
    };

    let current_exe = std::env::current_exe().map_err(InstallError::CurrentExe)?;
    let exe_path =
        fs::canonicalize(&current_exe).map_err(|source| InstallError::CanonicalizeExe {
            path: current_exe,
            source,
        })?;
    let service_content = render_service_content(&exe_path);

    let service_path = PathBuf::from(SYSTEMD_SERVICE_PATH);
    fs::write(&service_path, service_content).map_err(|source| InstallError::Io {
        path: service_path,
        source,
    })?;

    run_command("systemctl", &["daemon-reload"])?;
    run_command("systemctl", &["enable", SYSTEMD_SERVICE_NAME])?;

    Ok(InstallReport {
        config_path: LINUX_CONFIG_PATH,
        service_path: SYSTEMD_SERVICE_PATH,
        config_created,
    })
}

#[cfg(target_os = "linux")]
fn render_service_content(exe_path: &std::path::Path) -> String {
    let exe = quote_systemd_arg(&escape_systemd_value(&exe_path.display().to_string()));
    let config_path = quote_systemd_arg(&escape_systemd_value(LINUX_CONFIG_PATH));

    format!(
        "[Unit]
Description=AI Gateway Lite
After=network.target

[Service]
Type=simple
WorkingDirectory={LINUX_CONFIG_DIR}
ExecStart={exe} --config {config_path}
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
"
    )
}

#[cfg(target_os = "linux")]
fn escape_systemd_value(value: &str) -> String {
    value.replace('%', "%%")
}

#[cfg(target_os = "linux")]
fn quote_systemd_arg(value: &str) -> String {
    let needs_quote = value.chars().any(char::is_whitespace);
    if needs_quote {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

#[cfg(target_os = "linux")]
fn run_command(command: &str, args: &[&str]) -> Result<(), InstallError> {
    use std::process::Command;

    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|source| InstallError::CommandIo {
            command: format!("{command} {}", args.join(" ")),
            source,
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(InstallError::CommandFailed {
        command: format!("{command} {}", args.join(" ")),
        status_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn service_content_points_to_fixed_config_path() {
        let rendered = render_service_content(std::path::Path::new("/opt/ai-gw-lite/ai-gw-lite"));
        assert!(
            rendered.contains(
                "ExecStart=/opt/ai-gw-lite/ai-gw-lite --config /etc/ai_gw_lite/conf.yaml"
            )
        );
        assert!(rendered.contains("WorkingDirectory=/etc/ai_gw_lite"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn service_content_quotes_paths_with_space() {
        let rendered = render_service_content(std::path::Path::new("/opt/my gateway/ai-gw-lite"));
        assert!(rendered.contains(
            "ExecStart=\"/opt/my gateway/ai-gw-lite\" --config /etc/ai_gw_lite/conf.yaml"
        ));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_install_is_rejected() {
        assert!(matches!(
            run_install(),
            Err(InstallError::UnsupportedPlatform)
        ));
    }
}
