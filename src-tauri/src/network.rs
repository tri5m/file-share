use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr},
};
#[cfg(target_os = "macos")]
use std::process::Command;

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
        GetComputerNameExW, ComputerNameDnsHostname,
    };
    let mut buffer = [0_u16; 256];
    let mut length = buffer.len() as u32;
    // Use the Unicode system API without launching a console process.
    if unsafe { GetComputerNameExW(ComputerNameDnsHostname, buffer.as_mut_ptr(), &mut length) } == 0 {
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

fn is_shareable_interface(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let excluded_prefixes = [
        "lo", "utun", "awdl", "llw", "bridge", "gif", "stf", "p2p", "ipsec", "tap", "tun",
        "veth", "vethernet", "docker", "br-", "virbr", "ham", "wsl",
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
        "vethernet",
        "hyper-v",
        "hyperv",
        "wsl",
        "docker",
        "container",
        "default switch",
    ];
    !excluded_keywords
        .iter()
        .any(|keyword| name.contains(keyword))
}

#[cfg(test)]
mod tests {
    use super::is_shareable_interface;

    #[test]
    fn filters_windows_virtual_adapters() {
        for name in [
            "vEthernet (Default Switch)",
            "vEthernet (WSL)",
            "DockerNAT",
            "VirtualBox Host-Only Network",
            "Hyper-V Virtual Ethernet Adapter",
        ] {
            assert!(!is_shareable_interface(name), "{name}");
        }
        assert!(is_shareable_interface("Wi-Fi"));
        assert!(is_shareable_interface("Ethernet"));
    }
}
