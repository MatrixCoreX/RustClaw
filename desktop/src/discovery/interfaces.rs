// Platform adapter selection happens before any subnet request is sent.
pub(super) fn physical_interface(interface: &if_addrs::Interface) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/sys/class/net")
            .join(&interface.name)
            .join("device")
            .exists()
    }
    #[cfg(target_os = "macos")]
    {
        use system_configuration::network_configuration::{get_interfaces, SCNetworkInterfaceType};
        get_interfaces().iter().any(|native| {
            native
                .bsd_name()
                .is_some_and(|name| name.to_string() == interface.name)
                && matches!(
                    native.interface_type(),
                    Some(SCNetworkInterfaceType::Ethernet | SCNetworkInterfaceType::IEEE80211)
                )
        })
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};
        let Some(index) = interface.index.filter(|index| *index != 0) else {
            return false;
        };
        let mut row = MIB_IF_ROW2 {
            InterfaceIndex: index,
            ..Default::default()
        };
        // SAFETY: GetIfEntry2 synchronously initializes a correctly sized writable row.
        let status = unsafe { GetIfEntry2(&mut row) };
        status == 0 && windows_hardware_lan(row.Type, row.InterfaceAndOperStatusFlags._bitfield)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = interface;
        false
    }
}

#[cfg(any(target_os = "windows", test))]
fn windows_hardware_lan(kind: u32, flags: u8) -> bool {
    // MIB_IF_ROW2: Ethernet / IEEE80211, hardware, connector present, no filter.
    matches!(kind, 6 | 71) && flags & 0b111 == 0b101
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_requires_hardware_not_localized_adapter_names() {
        assert!(windows_hardware_lan(6, 0b101));
        assert!(windows_hardware_lan(71, 0b101));
        assert!(!windows_hardware_lan(6, 0b100)); // virtual Ethernet / VPN
        assert!(!windows_hardware_lan(6, 0b111)); // filter
        assert!(!windows_hardware_lan(24, 0b101)); // loopback
        assert!(!windows_hardware_lan(131, 0b101)); // tunnel
    }

    #[test]
    fn native_loopback_interfaces_are_never_scanned() {
        for interface in if_addrs::get_if_addrs()
            .unwrap()
            .iter()
            .filter(|i| i.is_loopback())
        {
            assert!(!physical_interface(interface));
        }
    }
}
