use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context};
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use claw_core::channel_i18n::{text_from_path, text_with_vars_from_path};
use claw_core::config::AppConfig;
use serde_json::json;
use tokio::process::Command;
use tracing::{error, info, warn};

#[derive(Clone, Default)]
struct BridgeHealth {
    running: bool,
    restarts: u64,
    last_exit_code: Option<i32>,
    last_error: Option<String>,
}

const WA_WEB_I18N_BRIDGE_PROCESS_EXITED_KEY: &str = "whatsapp_web.msg.bridge_process_exited";
const WA_WEB_I18N_BRIDGE_WAIT_FAILED_KEY: &str = "whatsapp_web.msg.bridge_wait_failed";
const WA_WEB_BRIDGE_PROCESS_EXITED_FALLBACK: &str = "bridge process exited";
const WA_WEB_BRIDGE_WAIT_FAILED_FALLBACK: &str = "bridge wait failed: {error}";

fn resolve_i18n_path(language: &str, configured_path: &str) -> String {
    let lang = language.trim();
    if !lang.is_empty() {
        let candidate = format!("configs/i18n/whatsapp-webd.{lang}.toml");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    configured_path.to_string()
}

fn wa_web_t(i18n_path: &str, key: &str, fallback: &str) -> String {
    text_from_path(i18n_path, key, fallback)
}

fn wa_web_t_with(i18n_path: &str, key: &str, vars: &[(&str, &str)], fallback: &str) -> String {
    text_with_vars_from_path(i18n_path, key, vars, fallback)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .with_target(false)
        .compact()
        .init();

    let config = AppConfig::load("configs/config.toml")?;
    if !config.whatsapp_web.enabled {
        warn!("whatsapp_webd disabled by config [whatsapp_web].enabled=false");
        return Ok(());
    }

    let workspace_root = std::env::current_dir().context("read current_dir failed")?;
    let bridge_path = workspace_root.join("services/wa-web-bridge/index.js");
    if !bridge_path.exists() {
        return Err(anyhow!(
            "wa-web bridge entry not found: {}",
            bridge_path.display()
        ));
    }

    let health = Arc::new(Mutex::new(BridgeHealth::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let i18n_path = resolve_i18n_path(
        &config.whatsapp_web.language,
        &config.whatsapp_web.i18n_path,
    );

    let supervisor_health = health.clone();
    let supervisor_stop = stop.clone();
    let supervisor_workspace = workspace_root.clone();
    let supervisor_bridge = bridge_path.clone();
    let supervisor_i18n = i18n_path.clone();
    tokio::spawn(async move {
        if let Err(err) = supervise_bridge(
            supervisor_workspace,
            supervisor_bridge,
            supervisor_i18n,
            supervisor_health,
            supervisor_stop,
        )
        .await
        {
            error!("bridge supervisor exited with error: {}", err);
        }
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .with_state(health.clone());

    info!(
        "whatsapp_webd started: wrapper_listen={} bridge={}",
        config.whatsapp_web.wrapper_listen,
        bridge_path.display()
    );
    let listener = tokio::net::TcpListener::bind(&config.whatsapp_web.wrapper_listen)
        .await
        .with_context(|| {
            format!(
                "bind wrapper listen failed: {}",
                config.whatsapp_web.wrapper_listen
            )
        })?;

    let server = axum::serve(listener, app);
    let shutdown = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut term_signal = signal(SignalKind::terminate()).ok();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async {
                    if let Some(sig) = term_signal.as_mut() {
                        let _ = sig.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    };
    tokio::select! {
        out = server => {
            if let Err(err) = out {
                error!("whatsapp_webd http server error: {}", err);
            }
        }
        _ = shutdown => {
            info!("whatsapp_webd received shutdown signal");
        }
    }

    stop.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(200)).await;
    Ok(())
}

async fn health_handler(State(health): State<Arc<Mutex<BridgeHealth>>>) -> Json<serde_json::Value> {
    let snapshot = health
        .lock()
        .map(|v| v.clone())
        .unwrap_or_else(|_| BridgeHealth::default());
    Json(json!({
        "ok": true,
        "bridge_running": snapshot.running,
        "restarts": snapshot.restarts,
        "last_exit_code": snapshot.last_exit_code,
        "last_error": snapshot.last_error,
    }))
}

async fn supervise_bridge(
    workspace_root: PathBuf,
    bridge_path: PathBuf,
    i18n_path: String,
    health: Arc<Mutex<BridgeHealth>>,
    stop: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let mut child = Command::new("node")
            .arg(&bridge_path)
            .current_dir(&workspace_root)
            .spawn()
            .with_context(|| format!("spawn node bridge failed: {}", bridge_path.display()))?;

        {
            if let Ok(mut h) = health.lock() {
                h.running = true;
                h.last_error = None;
            }
        }
        info!("wa-web bridge spawned pid={:?}", child.id());

        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = child.kill().await;
                if let Ok(mut h) = health.lock() {
                    h.running = false;
                }
                return Ok(());
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    let code = status.code();
                    warn!("wa-web bridge exited status={:?}", code);
                    if let Ok(mut h) = health.lock() {
                        h.running = false;
                        h.last_exit_code = code;
                        h.restarts = h.restarts.saturating_add(1);
                        h.last_error = Some(wa_web_t(
                            &i18n_path,
                            WA_WEB_I18N_BRIDGE_PROCESS_EXITED_KEY,
                            WA_WEB_BRIDGE_PROCESS_EXITED_FALLBACK,
                        ));
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    break;
                }
                Ok(None) => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(err) => {
                    if let Ok(mut h) = health.lock() {
                        h.running = false;
                        h.last_error = Some(wa_web_t_with(
                            &i18n_path,
                            WA_WEB_I18N_BRIDGE_WAIT_FAILED_KEY,
                            &[("error", &err.to_string())],
                            WA_WEB_BRIDGE_WAIT_FAILED_FALLBACK,
                        ));
                        h.restarts = h.restarts.saturating_add(1);
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    break;
                }
            }
        }
    }
    Ok(())
}
