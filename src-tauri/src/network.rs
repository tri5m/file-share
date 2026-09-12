use std::cmp::Reverse;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr},
};

#[derive(Debug, Clone)]
pub struct LanAddress {
    pub name: Option<String>,
    pub ip: String,
}

pub fn host_name() -> Option<String> {
    let name = system_host_name()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(unix)]
fn system_host_name() -> Option<String> {
    let mut buffer = [0_u8; 256];
    // The OS writes at most the provided buffer size. Require a terminator
    // instead of displaying a truncated name on systems with longer limits.
    if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
        return None;
    }
    let length = buffer.iter().position(|byte| *byte == 0)?;
    Some(String::from_utf8_lossy(&buffer[..length]).into_owned())
}

#[cfg(windows)]
fn system_host_name() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{
        ComputerNameDnsHostname, GetComputerNameExW,
    };
    let mut buffer = [0_u16; 256];
    let mut length = buffer.len() as u32;
    // Use the Unicode system API without launching a console process.
    if unsafe { GetComputerNameExW(ComputerNameDnsHostname, buffer.as_mut_ptr(), &mut length) } == 0
    {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

#[cfg(not(any(unix, windows)))]
fn system_host_name() -> Option<String> {
    None
}

pub fn lan_ipv4_addresses() -> Vec<LanAddress> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let names = interface_display_names();
    if let Ok(netifs) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in netifs {
            let display_name = names.get(&name).cloned();
            let Some(priority) = interface_priority(&name, display_name.as_deref()) else {
                continue;
            };
            if let IpAddr::V4(v4) = ip {
                if is_shareable_ipv4(v4) {
                    let value = v4.to_string();
                    if seen.insert(value.clone()) {
                        candidates.push((
                            priority,
                            LanAddress {
                                name: display_name.or_else(|| Some(name)),
                                ip: value,
                            },
                        ));
                    }
                }
            }
        }
    }
    // A physical Ethernet/Wi-Fi address wins over all virtual adapters. Keep
    // virtual addresses only as a fallback for machines that have no physical
    // interface (for example a VM or a host using a bridge-only network).
    let has_physical = candidates.iter().any(|(priority, _)| *priority >= 80);
    candidates.retain(|(priority, _)| !has_physical || *priority >= 80);
    candidates.sort_by_key(|(priority, address)| (Reverse(*priority), address.ip.clone()));
    let mut addresses: Vec<_> = candidates.into_iter().map(|(_, address)| address).collect();
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
    #[cfg(not(target_os = "macos"))]
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

fn is_shareable_ipv4(address: Ipv4Addr) -> bool {
    if address.is_loopback() || address.is_link_local() || address.is_unspecified() {
        return false;
    }

    true
}

fn interface_priority(interface: &str, display_name: Option<&str>) -> Option<u8> {
    let combined =
        format!("{} {}", interface, display_name.unwrap_or_default()).to_ascii_lowercase();
    let excluded = [
        "lo", "loopback", "utun", "awdl", "llw", "gif", "stf", "p2p", "ipsec", "tap", "tun",
    ];
    if excluded.iter().any(|prefix| combined.starts_with(prefix)) {
        return None;
    }
    let virtual_keywords = [
        "virtual",
        "vethernet",
        "hyper-v",
        "hyperv",
        "wsl",
        "docker",
        "container",
        "vmware",
        "virtualbox",
        "tailscale",
        "zerotier",
        "clash",
        "mihomo",
        "default switch",
        "veth",
        "virbr",
        "bridge",
        "ham",
    ];
    if virtual_keywords
        .iter()
        .any(|keyword| combined.contains(keyword))
    {
        return Some(10);
    }
    let physical_keywords = ["ethernet", "以太网", "wi-fi", "wifi", "wlan", "无线", "lan"];
    if physical_keywords
        .iter()
        .any(|keyword| combined.contains(keyword))
    {
        return Some(100);
    }
    Some(50)
}

#[cfg(test)]
mod tests {
    use super::interface_priority;

    #[test]
    fn filters_windows_virtual_adapters() {
        for name in [
            "vEthernet (Default Switch)",
            "vEthernet (WSL)",
            "DockerNAT",
            "VirtualBox Host-Only Network",
            "Hyper-V Virtual Ethernet Adapter",
        ] {
            assert_eq!(interface_priority(name, None), Some(10), "{name}");
        }
        assert_eq!(interface_priority("Wi-Fi", None), Some(100));
        assert_eq!(interface_priority("Ethernet", None), Some(100));
        assert_eq!(interface_priority("en0", Some("以太网")), Some(100));
    }
}
