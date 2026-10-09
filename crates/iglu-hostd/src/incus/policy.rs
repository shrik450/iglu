//! Keeps the workspace bridge's egress policy in place.
//!
//! Incus ACLs are allow-lists with a default action, so "the Internet except
//! private space" becomes one allow rule per prefix of the complement,
//! computed in the core.

use hyper::Method;
use iglu_domain::network::{allowed_ipv4, default_denied_ipv4};
use serde_json::{Value, json};

use super::client::{Incus, IncusError};

/// Creates or replaces the ACL and attaches it to the bridge with a default
/// egress action of reject. DNS to the bridge's own resolver stays allowed.
pub async fn ensure(
    incus: &Incus,
    network: &str,
    acl: &str,
    timeout: std::time::Duration,
) -> Result<(), IncusError> {
    let bridge: Value = incus.get(&format!("/1.0/networks/{network}")).await?;
    let gateway = bridge["config"]["ipv4.address"]
        .as_str()
        .and_then(|cidr| cidr.split('/').next())
        .ok_or_else(|| IncusError::Protocol(format!("network {network} has no IPv4 address")))?
        .to_owned();

    let allowed: Vec<String> = allowed_ipv4(&default_denied_ipv4())
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut egress: Vec<Value> = ["udp", "tcp"]
        .iter()
        .map(|protocol| {
            json!({
                "action": "allow",
                "state": "enabled",
                "description": "DNS to the bridge resolver",
                "destination": format!("{gateway}/32"),
                "protocol": protocol,
                "destination_port": "53",
            })
        })
        .collect();
    egress.push(json!({
        "action": "allow",
        "state": "enabled",
        "description": "the public Internet",
        "destination": allowed.join(","),
    }));
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
    tracing::info!(%network, %acl, prefixes = allowed.len(), "workspace egress policy in place");
    Ok(())
}
