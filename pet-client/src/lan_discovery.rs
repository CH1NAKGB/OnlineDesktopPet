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
                break Some(format!("ws://{}:{}/ws", ip, service.get_port()));
            }
            _ => continue,
        }
    };
    let _ = daemon.shutdown();
    found
}
