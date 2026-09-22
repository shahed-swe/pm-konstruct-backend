//! Address checks for outbound connections the user controls.
//!
//! A company's SMTP host is chosen by whoever configures the account, and the
//! server then connects to it. Without these checks an authenticated manager
//! could point it at the host's own network -- a cloud metadata endpoint, an
//! internal admin port -- and have the server reach it on their behalf. That
//! is server-side request forgery.
//!
//! Every branch here fails **closed**: an address that cannot be parsed is
//! treated as private, because an unparseable form is exactly what a bypass
//! attempt looks like.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::error::{DomainError, DomainResult};

/// Is this address one the server must not be steered at?
///
/// Covers loopback, the private ranges, link-local (which includes the cloud
/// metadata address), carrier-grade NAT and the unspecified address, for both
/// families and for IPv4 embedded in IPv6 in either the dotted or the
/// hexadecimal form.
#[must_use]
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4 address carried inside IPv6 is still that IPv4 address.
            // Both the mapped (::ffff:a.b.c.d) and the deprecated compatible
            // (::a.b.c.d) forms reach the same host.
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| compatible_v4(v6)) {
                return is_private_v4(v4);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7, unique local.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10, link local.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

const fn is_private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    match a {
        // 0.0.0.0/8 -- "this network", and 0.0.0.0 itself routes to localhost
        // on several stacks.
        0 => true,
        10 => true,
        127 => true,
        // 100.64.0.0/10, carrier-grade NAT.
        100 => b >= 64 && b <= 127,
        // 169.254.0.0/16, link local. The cloud metadata address lives here.
        169 => b == 254,
        172 => b >= 16 && b <= 31,
        192 => b == 168,
        _ => false,
    }
}

/// The deprecated `::a.b.c.d` form, which some stacks still route.
///
/// `Ipv6Addr::to_ipv4` would also return `::1` as `0.0.0.1`, so loopback is
/// excluded here and handled by the dedicated check.
fn compatible_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    if s[..6].iter().all(|g| *g == 0) && !(s[6] == 0 && s[7] <= 1) {
        return Some(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        ));
    }
    None
}

/// Hostnames that are never a legitimate SMTP relay.
const BLOCKED_HOSTNAMES: [&str; 3] = ["localhost", "metadata.google.internal", "169.254.169.254"];

/// Suffixes that only ever name something inside a private network.
const BLOCKED_SUFFIXES: [&str; 3] = [".local", ".internal", ".localhost"];

/// Checks a host before any DNS lookup.
///
/// A bare name with no dot is blocked: on a private network it resolves
/// through the search domain to an internal machine, and no public mail relay
/// is named that way.
#[must_use]
pub fn is_blocked_smtp_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    if h.is_empty() {
        return true;
    }
    if BLOCKED_HOSTNAMES.contains(&h.as_str()) {
        return true;
    }
    if BLOCKED_SUFFIXES.iter().any(|s| h.ends_with(s)) {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return is_private_ip(ip);
    }
    // Not an IP and not dotted: a search-domain name.
    !h.contains('.')
}

/// The port range a mail server can actually listen on.
pub fn validate_smtp_port(port: i32) -> DomainResult<()> {
    if !(1..=65535).contains(&port) {
        return Err(DomainError::invalid(
            "smtpPort",
            "must be between 1 and 65535",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn the_ipv4_private_ranges_are_blocked() {
        for s in [
            "10.0.0.1",
            "10.255.255.255",
            "127.0.0.1",
            "127.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "100.127.255.255",
        ] {
            assert!(is_private_ip(ip(s)), "{s} should be blocked");
        }
    }

    #[test]
    fn public_ipv4_is_allowed() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            // Just outside each blocked range.
            "172.15.0.1",
            "172.32.0.1",
            "192.169.0.1",
            "169.253.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "11.0.0.1",
            "126.0.0.1",
            "128.0.0.1",
        ] {
            assert!(!is_private_ip(ip(s)), "{s} should be allowed");
        }
    }

    #[test]
    fn ipv6_loopback_and_local_ranges_are_blocked() {
        for s in ["::1", "::", "fc00::1", "fd00::1", "fe80::1", "febf::1"] {
            assert!(is_private_ip(ip(s)), "{s} should be blocked");
        }
    }

    #[test]
    fn public_ipv6_is_allowed() {
        for s in ["2001:4860:4860::8888", "2606:4700:4700::1111", "fec0::1"] {
            assert!(!is_private_ip(ip(s)), "{s} should be allowed");
        }
    }

    #[test]
    fn ipv4_embedded_in_ipv6_is_still_that_address() {
        // The mapped form, dotted and hexadecimal.
        assert!(is_private_ip(ip("::ffff:127.0.0.1")));
        assert!(is_private_ip(ip("::ffff:7f00:1")));
        assert!(is_private_ip(ip("::ffff:169.254.169.254")));
        assert!(is_private_ip(ip("::ffff:10.0.0.1")));
        // And a mapped public address is still public.
        assert!(!is_private_ip(ip("::ffff:8.8.8.8")));
    }

    #[test]
    fn the_deprecated_compatible_form_is_handled_too() {
        // ::a.b.c.d, which some stacks still route.
        assert!(is_private_ip(ip("::127.0.0.1")));
        assert!(is_private_ip(ip("::169.254.169.254")));
        assert!(!is_private_ip(ip("::8.8.8.8")));
    }

    #[test]
    fn a_fully_written_out_mapped_address_is_caught() {
        assert!(is_private_ip(ip("0:0:0:0:0:ffff:127.0.0.1")));
        assert!(is_private_ip(ip("0000:0000:0000:0000:0000:ffff:7f00:0001")));
    }

    #[test]
    fn the_named_metadata_hosts_are_blocked() {
        for h in [
            "localhost",
            "LOCALHOST",
            "  localhost  ",
            "metadata.google.internal",
            "169.254.169.254",
        ] {
            assert!(is_blocked_smtp_host(h), "{h:?} should be blocked");
        }
    }

    #[test]
    fn private_network_suffixes_are_blocked() {
        for h in [
            "mail.local",
            "smtp.internal",
            "relay.localhost",
            "SMTP.INTERNAL",
        ] {
            assert!(is_blocked_smtp_host(h), "{h:?} should be blocked");
        }
    }

    #[test]
    fn a_bare_hostname_with_no_dot_is_blocked() {
        // It would resolve through the search domain to something internal,
        // and no public relay is named this way.
        for h in ["mailserver", "smtp", "db"] {
            assert!(is_blocked_smtp_host(h), "{h:?} should be blocked");
        }
    }

    #[test]
    fn real_mail_hosts_are_allowed() {
        for h in [
            "smtp.gmail.com",
            "smtp.office365.com",
            "mail.pmkonstruct.com.au",
            "8.8.8.8",
        ] {
            assert!(!is_blocked_smtp_host(h), "{h:?} should be allowed");
        }
    }

    #[test]
    fn an_empty_host_is_blocked() {
        assert!(is_blocked_smtp_host(""));
        assert!(is_blocked_smtp_host("   "));
    }

    #[test]
    fn an_ip_literal_host_is_checked_as_an_address() {
        assert!(is_blocked_smtp_host("127.0.0.1"));
        assert!(is_blocked_smtp_host("10.0.0.1"));
        assert!(is_blocked_smtp_host("::1"));
        assert!(is_blocked_smtp_host("::ffff:127.0.0.1"));
    }

    #[test]
    fn the_port_range_is_the_whole_valid_one() {
        assert!(validate_smtp_port(1).is_ok());
        assert!(validate_smtp_port(587).is_ok());
        assert!(validate_smtp_port(65535).is_ok());
        assert!(validate_smtp_port(0).is_err());
        assert!(validate_smtp_port(-1).is_err());
        assert!(validate_smtp_port(65536).is_err());
    }
}
