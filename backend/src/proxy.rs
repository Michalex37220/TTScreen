//! IP réelle du client derrière un proxy (équivalent de `--proxy-headers --forwarded-allow-ips` d'uvicorn).
//!
//! `X-Forwarded-For` n'est pris en compte que si la connexion directe vient d'une adresse de confiance
//! (`FORWARDED_ALLOW_IPS`, `*` = toutes). Sans cela, n'importe quel client pourrait choisir son IP
//! et contourner la limitation de débit.

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use ipnet::IpNet;
use std::net::{IpAddr, SocketAddr};

use crate::app::AppState;
use crate::deps::{latin1, ClientIp};

#[derive(Clone, Debug)]
pub struct TrustedProxies {
    all: bool,
    nets: Vec<IpNet>,
}

impl TrustedProxies {
    /// `*`, ou une liste séparée par des virgules d'adresses (`127.0.0.1`) et de réseaux (`10.0.0.0/8`).
    pub fn parse(spec: &str) -> Self {
        let spec = spec.trim();
        if spec == "*" {
            return TrustedProxies { all: true, nets: Vec::new() };
        }
        let mut nets = Vec::new();
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if let Ok(net) = part.parse::<IpNet>() {
                nets.push(net);
            } else if let Ok(ip) = part.parse::<IpAddr>() {
                nets.push(IpNet::from(ip));
            }
        }
        TrustedProxies { all: false, nets }
    }

    pub fn trusts(&self, ip: &IpAddr) -> bool {
        self.all || self.nets.iter().any(|n| n.contains(ip))
    }

    /// Adresse à retenir pour le client (même logique qu'uvicorn).
    pub fn resolve(&self, peer: Option<IpAddr>, forwarded_for: Option<&str>) -> String {
        let fallback = peer.map(|p| p.to_string()).unwrap_or_else(|| "inconnu".to_string());
        let peer_trusted = self.all || peer.map_or(false, |p| self.trusts(&p));
        if !peer_trusted {
            return fallback;
        }
        let Some(header) = forwarded_for else { return fallback };
        let hosts: Vec<&str> = header.split(',').map(str::trim).filter(|h| !h.is_empty()).collect();
        if hosts.is_empty() {
            return fallback;
        }
        if self.all {
            return hosts[0].to_string();
        }
        // Chaîne X-Forwarded-For : on remonte depuis la droite jusqu'au premier saut non fiable
        for host in hosts.iter().rev() {
            match host.parse::<IpAddr>() {
                Ok(ip) if self.trusts(&ip) => continue,
                _ => return (*host).to_string(),
            }
        }
        fallback
    }
}

/// Middleware le plus externe : calcule l'IP du client une fois pour toutes les couches suivantes.
pub async fn client_ip_middleware(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let forwarded = req.headers().get("x-forwarded-for").map(|v| latin1(v.as_bytes()));
    let ip = st.trusted_proxies.resolve(peer, forwarded.as_deref());
    req.extensions_mut().insert(ClientIp(ip));
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn untrusted_peer_cannot_spoof_its_address() {
        let t = TrustedProxies::parse("127.0.0.1");
        assert_eq!(t.resolve(ip("203.0.113.9"), Some("1.2.3.4")), "203.0.113.9");
    }

    #[test]
    fn trusted_peer_forwards_the_client_address() {
        let t = TrustedProxies::parse("127.0.0.1");
        assert_eq!(t.resolve(ip("127.0.0.1"), Some("198.51.100.4")), "198.51.100.4");
    }

    #[test]
    fn missing_header_falls_back_to_the_peer() {
        let t = TrustedProxies::parse("127.0.0.1");
        assert_eq!(t.resolve(ip("127.0.0.1"), None), "127.0.0.1");
        assert_eq!(t.resolve(ip("127.0.0.1"), Some("  ,  ")), "127.0.0.1");
    }

    #[test]
    fn wildcard_trusts_everyone_and_takes_the_first_hop() {
        let t = TrustedProxies::parse("*");
        assert_eq!(t.resolve(ip("172.18.0.3"), Some("198.51.100.4")), "198.51.100.4");
        assert_eq!(t.resolve(ip("172.18.0.3"), Some("198.51.100.4, 10.0.0.1")), "198.51.100.4");
        assert_eq!(t.resolve(None, Some("198.51.100.4")), "198.51.100.4");
    }

    #[test]
    fn chain_is_walked_from_the_right_skipping_trusted_hops() {
        let t = TrustedProxies::parse("10.0.0.0/8, 127.0.0.1");
        // client usurpé ("6.6.6.6") ajouté à gauche par l'attaquant : on retient le dernier saut non fiable
        assert_eq!(t.resolve(ip("10.1.1.1"), Some("6.6.6.6, 198.51.100.4, 10.2.2.2")), "198.51.100.4");
    }

    #[test]
    fn cidr_and_single_addresses_and_garbage() {
        let t = TrustedProxies::parse("192.168.0.0/16,::1, n'importe quoi,");
        assert!(t.trusts(&"192.168.5.5".parse().unwrap()));
        assert!(t.trusts(&"::1".parse().unwrap()));
        assert!(!t.trusts(&"192.169.0.1".parse().unwrap()));
        assert!(!TrustedProxies::parse("").trusts(&"127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn unknown_peer_is_reported_as_inconnu() {
        let t = TrustedProxies::parse("127.0.0.1");
        assert_eq!(t.resolve(None, Some("1.2.3.4")), "inconnu");
    }
}
