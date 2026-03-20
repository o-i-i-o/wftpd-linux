use anyhow::Result;
use std::sync::Arc;

use crate::core::config::Config;
use crate::core::logger::Logger;
use crate::core::users::UserManager;
use crate::core::file_logger::FileLogger;
use crate::core::quota::QuotaManager;
use crate::core::ftp_server::tls::TlsConfig;

pub async fn start_ftps_implicit_server(
    config: Arc<std::sync::Mutex<Config>>,
    user_manager: Arc<std::sync::Mutex<UserManager>>,
    logger: Arc<std::sync::Mutex<Logger>>,
    file_logger: Arc<std::sync::Mutex<FileLogger>>,
    quota_manager: Arc<QuotaManager>,
    tls_config: TlsConfig,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    let (bind_ip, ftps_port) = {
        let cfg = config.lock().unwrap();
        (cfg.ftp.bind_ip.clone(), cfg.ftp.ftps.implicit_ssl_port)
    };

    let bind_addr = format!("{}:{}", bind_ip, ftps_port);

    let listener = {
        use socket2::{Domain, Protocol, Socket, Type, SockAddr};
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        let addr: std::net::SocketAddr = bind_addr.parse()
            .map_err(|e| anyhow::anyhow!("Invalid bind address: {}", e))?;
        socket.bind(&SockAddr::from(addr))?;
        socket.listen(128)?;
        tokio::net::TcpListener::from_std(socket.into())
            .map_err(|e| anyhow::anyhow!("Failed to create tokio listener: {}", e))?
    };

    if let Ok(mut log) = logger.lock() {
        log.info("FTPS", &format!("FTPS server (implicit SSL) started on {}", bind_addr));
    }

    let tls_acceptor = match &tls_config.acceptor {
        Some(acceptor) => acceptor.clone(),
        None => {
            return Err(anyhow::anyhow!("TLS acceptor not available"));
        }
    };

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                break;
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((socket, peer_addr)) => {
                        let client_ip = peer_addr.ip().to_string();

                        if let Ok(mut log) = logger.lock() {
                            log.client_action(
                                "FTPS",
                                &format!("FTPS client connected from {}", client_ip),
                                &client_ip,
                                None,
                                "CONNECT",
                            );
                        }

                        let tls_stream = match tls_acceptor.accept(socket).await {
                            Ok(stream) => stream,
                            Err(e) => {
                                log::error!("FTPS TLS handshake failed: {}", e);
                                continue;
                            }
                        };

                        let config = Arc::clone(&config);
                        let user_manager = Arc::clone(&user_manager);
                        let logger = Arc::clone(&logger);
                        let file_logger = Arc::clone(&file_logger);
                        let quota_manager = Arc::clone(&quota_manager);

                        tokio::spawn(async move {
                            if let Err(e) = crate::core::ftp_server::session::handle_session_tls(
                                tls_stream,
                                config,
                                user_manager,
                                logger,
                                file_logger,
                                quota_manager,
                                client_ip,
                            ).await {
                                log::debug!("FTPS session error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        log::warn!("Failed to accept FTPS connection: {}", e);
                    }
                }
            }
        }
    }

    if let Ok(mut log) = logger.lock() {
        log.info("FTPS", "FTPS server stopped");
    }

    Ok(())
}
