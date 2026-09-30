//! Guest egress policy: the Internet, but not the host, the LAN or anything
//! else private.

use std::fmt;
use std::net::Ipv4Addr;

/// An IPv4 prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ipv4Cidr {
    network: u32,
    prefix: u8,
}

impl Ipv4Cidr {
    /// Builds a prefix, masking off host bits.
    pub const fn new(addr: Ipv4Addr, prefix: u8) -> Self {
        let prefix = if prefix > 32 { 32 } else { prefix };
        let bits = addr.to_bits();
        Self {
            network: bits & mask(prefix),
            prefix,
        }
    }

    pub const fn prefix(self) -> u8 {
        self.prefix
    }

    pub const fn address(self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.network)
    }

    const fn contains(self, other: Self) -> bool {
        other.prefix >= self.prefix && other.network & mask(self.prefix) == self.network
    }

    fn halves(self) -> Option<(Self, Self)> {
        if self.prefix == 32 {
            return None;
        }
        let prefix = self.prefix + 1;
        let high_bit = 1u32 << (32 - prefix);
        Some((
            Self {
                network: self.network,
                prefix,
            },
            Self {
                network: self.network | high_bit,
                prefix,
            },
        ))
    }
}

const fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}

impl fmt::Display for Ipv4Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address(), self.prefix)
    }
}

/// Destinations guests may never reach: this network, private and shared
/// address space (which covers VPN and Tailscale ranges), loopback,
/// link-local (which covers cloud metadata), documentation, benchmarking,
/// multicast and reserved space.
pub const DENIED_IPV4: &[(Ipv4Addr, u8)] = &[
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

/// The smallest set of prefixes covering exactly the IPv4 space outside
/// `denied`. Incus ACLs only allow, so the policy is expressed as this set.
pub fn allowed_ipv4(denied: &[Ipv4Cidr]) -> Vec<Ipv4Cidr> {
    let mut allowed = Vec::new();
    let mut pending = vec![Ipv4Cidr::new(Ipv4Addr::UNSPECIFIED, 0)];
    while let Some(block) = pending.pop() {
        if denied.iter().any(|d| d.contains(block)) {
            continue;
        }
        let overlaps = denied.iter().any(|d| block.contains(*d));
        match (overlaps, block.halves()) {
            (false, _) | (true, None) => allowed.push(block),
            (true, Some((low, high))) => {
                pending.push(high);
                pending.push(low);
            }
        }
    }
    allowed.sort();
    allowed
}

/// The default denied set as prefixes.
pub fn default_denied_ipv4() -> Vec<Ipv4Cidr> {
    DENIED_IPV4
        .iter()
        .map(|&(addr, prefix)| Ipv4Cidr::new(addr, prefix))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed_contains(allowed: &[Ipv4Cidr], addr: Ipv4Addr) -> bool {
        allowed
            .iter()
            .any(|block| block.contains(Ipv4Cidr::new(addr, 32)))
    }

    #[test]
    fn public_addresses_are_allowed_and_private_ones_are_not() {
        let allowed = allowed_ipv4(&default_denied_ipv4());
        for public in [
            Ipv4Addr::new(1, 1, 1, 1),
            Ipv4Addr::new(140, 82, 112, 3),
            Ipv4Addr::new(8, 8, 8, 8),
        ] {
            assert!(allowed_contains(&allowed, public), "{public}");
        }
        for private in [
            Ipv4Addr::new(10, 1, 2, 3),
            Ipv4Addr::new(192, 168, 1, 1),
            Ipv4Addr::new(172, 20, 0, 1),
            Ipv4Addr::new(100, 100, 100, 100),
            Ipv4Addr::new(169, 254, 169, 254),
            Ipv4Addr::new(127, 0, 0, 1),
            Ipv4Addr::new(255, 255, 255, 255),
        ] {
            assert!(!allowed_contains(&allowed, private), "{private}");
        }
    }

    #[test]
    fn the_complement_is_exact() {
        let denied = default_denied_ipv4();
        let allowed = allowed_ipv4(&denied);
        let size = |blocks: &[Ipv4Cidr]| -> u64 {
            blocks
                .iter()
                .map(|b| 1u64 << (32 - u32::from(b.prefix())))
                .sum()
        };
        assert_eq!(size(&allowed) + size(&denied), 1u64 << 32);
        for (i, a) in allowed.iter().enumerate() {
            assert!(!denied.iter().any(|d| d.contains(*a) || a.contains(*d)));
            assert!(
                !allowed[i + 1..]
                    .iter()
                    .any(|b| a.contains(*b) || b.contains(*a))
            );
        }
    }

    #[test]
    fn host_bits_are_masked() {
        assert_eq!(
            Ipv4Cidr::new(Ipv4Addr::new(10, 1, 2, 3), 8).to_string(),
            "10.0.0.0/8"
        );
    }
}
