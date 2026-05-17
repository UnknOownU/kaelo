use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone)]
pub struct SsrfError {
    pub url: String,
    pub reason: String,
}

impl std::fmt::Display for SsrfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SSRF protection: {} ({})", self.reason, self.url)
    }
}

impl std::error::Error for SsrfError {}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => is_private_ipv4(ipv4),
        IpAddr::V6(ipv6) => is_private_ipv6(ipv6),
    }
}

fn is_private_ipv4(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    // Loopback: 127.0.0.0/8
    if octets[0] == 127 {
        return true;
    }
    // Private: 10.0.0.0/8
    if octets[0] == 10 {
        return true;
    }
    // Private: 172.16.0.0/12
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return true;
    }
    // Private: 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return true;
    }
    // Link-local: 169.254.0.0/16
    if octets[0] == 169 && octets[1] == 254 {
        return true;
    }
    // "This" network: 0.0.0.0/8
    if octets[0] == 0 {
        return true;
    }
    // Broadcast: 255.255.255.255
    if ip.is_broadcast() {
        return true;
    }
    false
}

fn is_private_ipv6(ip: &Ipv6Addr) -> bool {
    // Loopback: ::1
    if ip.is_loopback() {
        return true;
    }
    // Link-local: fe80::/10
    let segments = ip.segments();
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // Unique local: fc00::/7
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // IPv4-mapped: ::ffff:0:0/96
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_private_ipv4(&v4);
    }
    false
}

pub fn check_ssrf(url: &str) -> Result<(), SsrfError> {
    let parsed = url::Url::parse(url).map_err(|e| SsrfError {
        url: url.to_string(),
        reason: format!("invalid URL: {e}"),
    })?;

    let host = parsed.host_str().ok_or_else(|| SsrfError {
        url: url.to_string(),
        reason: "no host in URL".to_string(),
    })?;

    // Try parsing as IP directly
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            return Err(SsrfError {
                url: url.to_string(),
                reason: format!("private IP address: {ip}"),
            });
        }
        return Ok(());
    }

    // For hostnames, we do a simple DNS resolution check
    // Note: This is a best-effort check. DNS rebinding attacks are not prevented.
    if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&format!("{host}:0")) {
        for addr in addrs {
            if is_private_ip(&addr.ip()) {
                return Err(SsrfError {
                    url: url.to_string(),
                    reason: format!("hostname resolves to private IP: {}", addr.ip()),
                });
            }
        }
    }

    Ok(())
}

pub fn check_ssrf_with_config(url: &str, allow_private: bool) -> Result<(), SsrfError> {
    if allow_private {
        return Ok(());
    }
    check_ssrf(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_localhost() {
        assert!(check_ssrf("http://127.0.0.1/secret").is_err());
        assert!(check_ssrf("http://127.0.0.1:8080/api").is_err());
        assert!(check_ssrf("http://localhost/path").is_err());
    }

    #[test]
    fn test_block_private_10() {
        assert!(check_ssrf("http://10.0.0.1/").is_err());
        assert!(check_ssrf("http://10.255.255.255/").is_err());
    }

    #[test]
    fn test_block_private_172() {
        assert!(check_ssrf("http://172.16.0.1/").is_err());
        assert!(check_ssrf("http://172.31.255.255/").is_err());
    }

    #[test]
    fn test_block_private_192() {
        assert!(check_ssrf("http://192.168.1.1/").is_err());
        assert!(check_ssrf("http://192.168.0.1/").is_err());
    }

    #[test]
    fn test_block_aws_metadata() {
        assert!(check_ssrf("http://169.254.169.254/metadata").is_err());
    }

    #[test]
    fn test_allow_public_ips() {
        assert!(check_ssrf("https://8.8.8.8").is_ok());
        assert!(check_ssrf("https://1.1.1.1").is_ok());
    }

    #[test]
    fn test_allow_public_domains() {
        assert!(check_ssrf("https://example.com").is_ok());
        assert!(check_ssrf("https://github.com").is_ok());
    }

    #[test]
    fn test_config_bypass() {
        assert!(check_ssrf_with_config("http://127.0.0.1/", true).is_ok());
        assert!(check_ssrf_with_config("http://127.0.0.1/", false).is_err());
    }

    #[test]
    fn test_invalid_url() {
        assert!(check_ssrf("not-a-url").is_err());
    }
}
