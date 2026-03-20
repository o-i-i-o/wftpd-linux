use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

pub type PassiveListenerMap = Arc<Mutex<HashMap<u16, Arc<Mutex<Option<TcpListener>>>>>>;

pub async fn find_available_passive_port(
    passive_listeners: &PassiveListenerMap,
    port_min: u16,
    port_max: u16,
) -> Result<u16> {
    let listeners = passive_listeners.lock().await;

    for port in port_min..=port_max {
        if !listeners.contains_key(&port) {
            return Ok(port);
        }
    }

    anyhow::bail!(
        "No available passive ports in range {}-{}",
        port_min,
        port_max
    )
}

pub async fn get_data_connection(
    passive_mode: bool,
    data_port: Option<u16>,
    data_addr: &Option<String>,
    remote_ip: &str,
    passive_listeners: &PassiveListenerMap,
    data_timeout_secs: u64,
) -> Result<tokio::net::TcpStream> {
    let port = match data_port {
        Some(p) => p,
        None => anyhow::bail!("No data port specified"),
    };

    let stream = if passive_mode {
        let listener_arc = {
            let listeners = passive_listeners.lock().await;
            listeners.get(&port).cloned()
        };

        if let Some(listener_arc) = listener_arc {
            let mut listener_guard = listener_arc.lock().await;
            if let Some(listener) = listener_guard.take() {
                let accept_timeout = Duration::from_secs(data_timeout_secs);
                match timeout(accept_timeout, listener.accept()).await {
                    Ok(Ok((stream, _))) => stream,
                    Ok(Err(e)) => anyhow::bail!("Failed to accept passive connection: {}", e),
                    Err(_) => anyhow::bail!("Timeout waiting for passive connection"),
                }
            } else {
                anyhow::bail!("No passive listener")
            }
        } else {
            anyhow::bail!("No passive listener")
        }
    } else if let Some(addr) = data_addr {
        let connect_timeout = Duration::from_secs(data_timeout_secs);
        match timeout(connect_timeout, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => anyhow::bail!("Failed to connect to {}: {}", addr, e),
            Err(_) => anyhow::bail!("Timeout connecting to {}", addr),
        }
    } else {
        let connect_addr = format!("{}:{}", remote_ip, port);
        let connect_timeout = Duration::from_secs(data_timeout_secs);
        match timeout(connect_timeout, tokio::net::TcpStream::connect(&connect_addr)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => anyhow::bail!("Failed to connect to {}: {}", connect_addr, e),
            Err(_) => anyhow::bail!("Timeout connecting to {}", connect_addr),
        }
    };

    Ok(stream)
}

 pub async fn create_passive_listener(
    bind_ip: &str,
    passive_port: u16,
) -> Result<TcpListener> {
    let bind_addr = format!("{}:{}", bind_ip, passive_port);
    let listener = TcpListener::bind(&bind_addr).await?;
    Ok(listener)
}
