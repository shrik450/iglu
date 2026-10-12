//! Keeps the workspace bridge's egress policy in place: the Internet, but
//! not the host, the LAN or anything else private. Incus applies an ACL's
//! reject rules before its allow rules, whatever their order, and lets DNS
//! and DHCP reach the bridge before any of them.

use hyper::Method;
use serde_json::{Value, json};

use super::client::{Incus, IncusError};

/// Destinations guests may never reach: this network, private and shared
/// address space (which covers VPN and Tailscale ranges), loopback,
/// link-local (which covers cloud metadata), documentation, benchmarking,
/// multicast and reserved space.
const DENIED: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/4",
    "240.0.0.0/4",
];

/// Creates or replaces the ACL and attaches it to the bridge, rejecting
/// whatever it doesn't allow, which with IPv6 off is nothing else.
pub async fn ensure(
    incus: &Incus,
    network: &str,
    acl: &str,
    timeout: std::time::Duration,
) -> Result<(), IncusError> {
    let bridge: Value = incus.get(&format!("/1.0/networks/{network}")).await?;
    let egress = json!([
        {
            "action": "reject",
            "state": "enabled",
            "description": "the host and private networks",
            "destination": DENIED.join(","),
        },
        {
            "action": "allow",
            "state": "enabled",
            "description": "the public Internet",
            "destination": "0.0.0.0/0",
        },
    ]);
    let rules = json!({
        "description": "iglu workspace egress: the Internet, never the host or private networks",
        "egress": egress,
        "ingress": [],
        "config": {},
    });

    match incus
        .get::<Value>(&format!("/1.0/network-acls/{acl}"))
        .await
    {
        Ok(_) => {
            incus
                .run(
                    Method::PUT,
                    &format!("/1.0/network-acls/{acl}"),
                    Some(&rules),
                    timeout,
                )
                .await?;
        }
        Err(error) if error.is_not_found() => {
            let mut create = rules.clone();
            create["name"] = json!(acl);
            incus
                .run(Method::POST, "/1.0/network-acls", Some(&create), timeout)
                .await?;
        }
        Err(error) => return Err(error),
    }

    let mut updated = bridge.clone();
    updated["config"]["security.acls"] = json!(acl);
    updated["config"]["security.acls.default.egress.action"] = json!("reject");
    updated["config"]["security.acls.default.ingress.action"] = json!("allow");
    updated["config"]["ipv6.address"] = json!("none");
    let put = json!({ "config": updated["config"], "description": updated["description"] });
    incus
        .run(
            Method::PUT,
            &format!("/1.0/networks/{network}"),
            Some(&put),
            timeout,
        )
        .await?;
    tracing::info!(%network, %acl, "workspace egress policy in place");
    Ok(())
}
