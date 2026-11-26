pub mod app;
pub mod config;
pub mod crypto;
pub mod executor;
pub mod flows;
pub mod icons;
pub mod mcp;
pub mod state;
pub mod store;

use std::sync::Arc;

/// Levanta el servidor en 127.0.0.1 (puerto 0 = uno libre). Devuelve el puerto real.
/// En modo público, `MARTINO_BIND=0.0.0.0` lo abre fuera de la máquina (p. ej. en Fly); entonces la
/// llave se pide siempre, también a quien diga ser `127.0.0.1`.
pub async fn serve(core: Arc<state::Core>) -> std::io::Result<(u16, tokio::task::JoinHandle<()>)> {
    let (port, public) = { let c = core.cfg.read().unwrap(); (c.port, c.public_key.is_some()) };
    let bind = std::env::var("MARTINO_BIND").ok().filter(|b| public && !b.trim().is_empty()).unwrap_or_else(|| "127.0.0.1".into());
    app::set_exposed(bind != "127.0.0.1", public && executor::demo());
    let listener = tokio::net::TcpListener::bind((bind.as_str(), port)).await?;
    let port = listener.local_addr()?.port();
    core.cfg.write().unwrap().port = port;
    let app = app::router(state::Hub(Arc::new(std::sync::RwLock::new(core))));
    let h = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((port, h))
}
