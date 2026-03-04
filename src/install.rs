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
const INSTALL_BIN_PATH: &str = "/usr/local/bin/ai-gw-lite";

#[cfg(target_os = "linux")]
const DEFAULT_CONFIG_TEMPLATE: &str = r#"listen: "0.0.0.0:8080"

gateway_auth:
  token_sources:
    - type: "authorization_bearer"

# 数据目录配置（相对于 conf.yaml 的路径）
# 路由、API Key 和封禁规则将从此目录加载
data_dir: "./data"

# CORS 配置
cors:
  enabled: false
  allow_origins: []
  allow_headers: []
  allow_methods: []
  expose_headers: []

# 限流配置
rate_limit:
  per_minute: 120

# 并发控制配置
concurrency:
  downstream_max_inflight: 100
  upstream_per_key_max_inflight: 8

# 可观测性配置
observability:
  logging:
    level: "info"
    format: "json"
    to_stdout: true
    file:
      enabled: true
      dir: "./logs"
      prefix: "ai-gw-lite"
      rotation: "daily"
      max_files: 7
  metrics:
    enabled: true
    path: "/metrics"
    token: "${GW_METRICS_TOKEN}"
  tracing:
    enabled: false
    sample_ratio: 0.05

# Admin 管理界面配置
admin:
  enabled: true
  token: "${ADMIN_TOKEN}"
"#;

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub config_path: &'static str,
    pub service_path: &'static str,
    pub bin_path: &'static str,
    pub config_created: bool,
    pub service_was_running: bool,
    pub bin_updated: bool,
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

    // 1. 检查服务是否已存在且正在运行
    let service_exists = Path::new(SYSTEMD_SERVICE_PATH).exists();
    let service_was_running =
        service_exists && is_service_active().map_err(|e| InstallError::CommandIo {
            command: "systemctl is-active".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
        })?;

    // 2. 如果服务正在运行，先停止服务
    if service_was_running {
        println!("检测到服务正在运行，正在停止...");
        run_command("systemctl", &["stop", SYSTEMD_SERVICE_NAME])?;
        println!("服务已停止");
    }

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

    // 3. 创建 data 目录结构和示例配置文件
    create_data_dir_structure(config_dir)?;

    // 4. 获取当前可执行文件路径并复制到安装位置
    let current_exe = std::env::current_exe().map_err(InstallError::CurrentExe)?;
    let exe_path =
        fs::canonicalize(&current_exe).map_err(|source| InstallError::CanonicalizeExe {
            path: current_exe,
            source,
        })?;

    // 复制二进制文件到安装路径
    let bin_path = Path::new(INSTALL_BIN_PATH);
    let bin_updated = if exe_path != bin_path {
        println!("正在更新二进制文件...");
        fs::copy(&exe_path, bin_path).map_err(|source| InstallError::Io {
            path: bin_path.to_path_buf(),
            source,
        })?;
        // 设置可执行权限
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(bin_path).map_err(|source| InstallError::Io {
                path: bin_path.to_path_buf(),
                source,
            })?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(bin_path, perms).map_err(|source| InstallError::Io {
                path: bin_path.to_path_buf(),
                source,
            })?;
        }
        println!("二进制文件已更新到: {}", INSTALL_BIN_PATH);
        true
    } else {
        println!("当前已在安装路径运行，跳过二进制复制");
        false
    };

    // 4. 使用固定的安装路径渲染服务文件
    let service_content = render_service_content(Path::new(INSTALL_BIN_PATH));

    let service_path = PathBuf::from(SYSTEMD_SERVICE_PATH);
    fs::write(&service_path, service_content).map_err(|source| InstallError::Io {
        path: service_path,
        source,
    })?;

    // 5. 重新加载 systemd 配置
    run_command("systemctl", &["daemon-reload"])?;

    // 6. 启用服务
    run_command("systemctl", &["enable", SYSTEMD_SERVICE_NAME])?;

    // 7. 启动服务
    println!("正在启动服务...");
    run_command("systemctl", &["start", SYSTEMD_SERVICE_NAME])?;
    println!("服务已启动");

    Ok(InstallReport {
        config_path: LINUX_CONFIG_PATH,
        service_path: SYSTEMD_SERVICE_PATH,
        bin_path: INSTALL_BIN_PATH,
        config_created,
        service_was_running,
        bin_updated,
    })
}

#[cfg(target_os = "linux")]
fn create_data_dir_structure(config_dir: &std::path::Path) -> Result<(), InstallError> {
    use std::fs;

    let data_dir = config_dir.join("data");
    let routes_dir = data_dir.join("routes");
    let apikeys_dir = data_dir.join("apikeys");

    // 创建目录结构
    fs::create_dir_all(&routes_dir).map_err(|source| InstallError::Io {
        path: routes_dir.clone(),
        source,
    })?;
    fs::create_dir_all(&apikeys_dir).map_err(|source| InstallError::Io {
        path: apikeys_dir.clone(),
        source,
    })?;

    // 创建示例路由配置（如果不存在）
    let example_route_path = routes_dir.join("openai.yaml");
    if !example_route_path.exists() {
        let route_template = r#"id: "openai"
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
        fs::write(&example_route_path, route_template).map_err(|source| InstallError::Io {
            path: example_route_path,
            source,
        })?;
    }

    // 创建示例 API Key 配置（如果不存在）
    let example_key_path = apikeys_dir.join("default.yaml");
    if !example_key_path.exists() {
        let key_template = r#"id: "default"
key: "${GW_TOKEN}"
enabled: true
remark: "默认 API Key"
route_ids:
  - "openai"
rate_limit:
  per_minute: 60
"#;
        fs::write(&example_key_path, key_template).map_err(|source| InstallError::Io {
            path: example_key_path,
            source,
        })?;
    }

    // 创建示例 ban_rules 配置（如果不存在）
    let ban_rules_path = data_dir.join("ban_rules.yaml");
    if !ban_rules_path.exists() {
        let ban_rules_template = r#"rules:
  - id: "rule_1"
    name: "高错误率封禁"
    condition:
      type: "error_rate"
      window_secs: 300
      threshold: 0.5
      min_requests: 10
    ban_duration_secs: 3600
    enabled: true
    trigger_count_threshold: 3
    trigger_window_secs: 3600
"#;
        fs::write(&ban_rules_path, ban_rules_template).map_err(|source| InstallError::Io {
            path: ban_rules_path,
            source,
        })?;
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn is_service_active() -> Result<bool, std::io::Error> {
    use std::process::Command;

    let output = Command::new("systemctl")
        .args(["is-active", SYSTEMD_SERVICE_NAME])
        .output()?;

    // systemctl is-active 返回 0 表示服务正在运行
    Ok(output.status.success())
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
        let rendered = render_service_content(std::path::Path::new("/usr/local/bin/ai-gw-lite"));
        assert!(
            rendered.contains(
                "ExecStart=/usr/local/bin/ai-gw-lite --config /etc/ai_gw_lite/conf.yaml"
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
