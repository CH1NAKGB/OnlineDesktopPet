use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

const LAN_SERVICE_TYPE: &str = "_chicklink._tcp.local.";

/// Finds one local relay without contacting the public internet.
/// The caller keeps the public URL as a fallback when no local service responds.
pub fn find_relay(timeout: Duration) -> Option<String> {
    let daemon = ServiceDaemon::new().ok()?;
    let receiver = daemon.browse(LAN_SERVICE_TYPE).ok()?;
    let deadline = Instant::now() + timeout;
    let found = loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        match receiver.recv_timeout(remaining).ok()? {
            ServiceEvent::ServiceResolved(service) => {
                let addresses = service.get_addresses_v4();
                let ip = addresses.iter().next()?;
                let scheme = service.get_property_val_str("scheme").unwrap_or("ws");
                let host = service
                    .get_property_val_str("host")
                    .filter(|host| !host.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| ip.to_string());
                let port = service
                    .get_property_val_str("port")
                    .and_then(|value| value.parse::<u16>().ok())
                    .unwrap_or_else(|| service.get_port());
                let path = service.get_property_val_str("path").unwrap_or("/ws");
                break Some(format!("{scheme}://{host}:{port}{path}"));
            }
            _ => continue,
        }
    };
    let _ = daemon.shutdown();
    found
}
