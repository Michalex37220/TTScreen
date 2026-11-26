//! Point d'entrée : `ttscreen` démarre le serveur, `ttscreen healthcheck` sert de sonde Docker
//! (l'image finale ne contient ni Python, ni curl).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use ttscreen::{create_app, Settings};

fn port() -> u16 {
    std::env::var("TTSCREEN_PORT").ok().and_then(|p| p.trim().parse().ok()).unwrap_or(8000)
}

/// Interroge `/api/health` sur la machine locale ; code de sortie 0 si 200.
fn healthcheck() -> anyhow::Result<()> {
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port())), Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    if reply.starts_with("HTTP/1.1 200") || reply.starts_with("HTTP/1.0 200") {
        Ok(())
    } else {
        anyhow::bail!("sonde de santé : réponse inattendue : {}", reply.lines().next().unwrap_or(""))
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("arrêt demandé, fermeture propre…");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        return healthcheck();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn,sea_orm=warn")),
        )
        .init();

    let settings = Settings::from_env().map_err(anyhow::Error::msg)?;
    let host = std::env::var("TTSCREEN_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let app = create_app(settings).await?;

    let listener = tokio::net::TcpListener::bind((host.as_str(), port())).await?;
    tracing::info!("TTScreen écoute sur http://{}", listener.local_addr()?);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}
