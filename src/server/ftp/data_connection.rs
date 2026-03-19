use anyhow::Result;
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type PassiveListenerMap = Arc<Mutex<HashMap<u16, Arc<Mutex<Option<TcpListener>>>>>>;

pub fn find_available_passive_port(
    passive_listeners: &PassiveListenerMap,
    port_min: u16,
    port_max: u16,
) -> Result<u16> {
    let listeners = passive_listeners.lock().unwrap();

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

pub fn get_data_connection(
    passive_mode: bool,
    data_port: Option<u16>,
    data_addr: &Option<String>,
    remote_ip: &str,
    passive_listeners: &PassiveListenerMap,
    data_timeout_secs: u64,
) -> Result<TcpStream> {
    let port = match data_port {
        Some(p) => p,
        None => anyhow::bail!("No data port specified"),
    };

    let stream = if passive_mode {
        let listener_arc = {
            let listeners = passive_listeners.lock().unwrap();
            listeners.get(&port).cloned()
        };

        if let Some(listener_arc) = listener_arc {
            let mut listener_guard = listener_arc.lock().unwrap();
            if let Some(listener) = listener_guard.take() {
                listener.set_nonblocking(false)?;
                listener.accept().map(|(s, _)| s)
                    .map_err(|e| anyhow::anyhow!("Failed to accept passive connection: {}", e))
            } else {
                anyhow::bail!("No passive listener")
            }
        } else {
            anyhow::bail!("No passive listener")
        }
    } else if let Some(addr) = data_addr {
        TcpStream::connect(addr)
            .map_err(|e| anyhow::anyhow!("Failed to connect to {}: {}", addr, e))
    } else {
        TcpStream::connect(format!("{}:{}", remote_ip, port))
            .map_err(|e| anyhow::anyhow!("Failed to connect to {}:{}: {}", remote_ip, port, e))
    }?;

    if data_timeout_secs > 0 {
        let timeout = Some(Duration::from_secs(data_timeout_secs));
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
    }

    Ok(stream)
}

pub fn create_passive_listener(
    bind_ip: &str,
    passive_port: u16,
) -> Result<TcpListener> {
    let listener = TcpListener::bind(format!("{}:{}", bind_ip, passive_port))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}
