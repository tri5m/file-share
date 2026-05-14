use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr},
};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct LanAddress {
    pub name: Option<String>,
    pub ip: String,
}

pub fn lan_ipv4_addresses() -> Vec<LanAddress> {
    let mut seen = HashSet::new();
    let mut addresses = Vec::new();
    let names = interface_display_names();
    if let Ok(netifs) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in netifs {
            if !is_shareable_interface(&name) {
                continue;
            }
            if let IpAddr::V4(v4) = ip {
                if is_shareable_ipv4(v4) {
                    let value = v4.to_string();
                    if seen.insert(value.clone()) {
                        addresses.push(LanAddress {
                            name: names.get(&name).cloned().or_else(|| Some(name)),
                            ip: value,
                        });
                    }
                }
            }
        }
    }
    if addresses.is_empty() {
        if let Ok(IpAddr::V4(v4)) = local_ip_address::local_ip() {
            if is_shareable_ipv4(v4) {
                addresses.push(LanAddress {
                    name: None,
                    ip: v4.to_string(),
                });
            }
        }
    }
    addresses
}

fn interface_display_names() -> HashMap<String, String> {
    #[cfg(target_os = "macos")]
    {
        macos_interface_display_names()
    }
    #[cfg(target_os = "windows")]
    {
        windows_interface_display_names()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        HashMap::new()
    }
}

#[cfg(target_os = "macos")]
fn macos_interface_display_names() -> HashMap<String, String> {
    let output = Command::new("networksetup")
        .arg("-listallhardwareports")
        .output();
    let Ok(output) = output else {
        return HashMap::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut names = HashMap::new();
    let mut current_name: Option<String> = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("Hardware Port: ") {
            current_name = Some(name.trim().to_string());
        } else if let Some(device) = line.strip_prefix("Device: ") {
            if let Some(name) = current_name.take() {
                names.insert(device.trim().to_string(), name);
            }
        }
    }
    names
}

#[cfg(target_os = "windows")]
fn windows_interface_display_names() -> HashMap<String, String> {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut command = Command::new("powershell");
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "Get-NetAdapter | ForEach-Object { \"$($_.InterfaceDescription)`t$($_.Name)`t$($_.InterfaceAlias)\" }",
        ]);
    let output = command.output();
    let Ok(output) = output else {
        return HashMap::new();
    };
    let mut names = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split('\t');
        let description = parts.next().unwrap_or_default().trim();
        let name = parts.next().unwrap_or_default().trim();
        let alias = parts.next().unwrap_or_default().trim();
        let display = if alias.is_empty() { name } else { alias };
        if display.is_empty() {
            continue;
        }
        if !description.is_empty() {
            names.insert(description.to_string(), display.to_string());
        }
        if !name.is_empty() {
            names.insert(name.to_string(), display.to_string());
        }
    }
    names
}

fn is_shareable_ipv4(address: Ipv4Addr) -> bool {
    if address.is_loopback() || address.is_link_local() || address.is_unspecified() {
        return false;
    }

    true
}

fn is_shareable_interface(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let excluded_prefixes = [
        "lo", "utun", "awdl", "llw", "bridge", "gif", "stf", "p2p", "ipsec", "tap", "tun",
    ];
    if excluded_prefixes
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return false;
    }

    let excluded_keywords = [
        "loopback",
        "virtual",
        "vmware",
        "virtualbox",
        "hyper-v",
        "tailscale",
        "zerotier",
        "clash",
        "mihomo",
    ];
    !excluded_keywords
        .iter()
        .any(|keyword| name.contains(keyword))
}
