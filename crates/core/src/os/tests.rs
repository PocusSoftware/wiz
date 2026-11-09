//! OS detection from command output.

use super::*;

#[test]
fn os_release_id_wins() {
    let out =
        "PRETTY_NAME=\"Ubuntu 24.04 LTS\"\nNAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nLinux\n";
    assert_eq!(parse(out), Some("ubuntu"));
}

#[test]
fn ids_are_mapped_and_unquoted() {
    assert_eq!(parse("ID=\"rhel\"\nLinux"), Some("redhat"));
    assert_eq!(parse("ID='pop'\nLinux"), Some("popos"));
    assert_eq!(parse("ID=arch\nLinux"), Some("archlinux"));
    assert_eq!(parse("ID=nixos\nLinux"), Some("nixos"));
}

#[test]
fn derivative_falls_back_to_id_like() {
    let out = "ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"\nLinux";
    assert_eq!(parse(out), Some("opensuse"));
    assert_eq!(
        parse("ID=someclone\nID_LIKE=\"rhel centos fedora\"\nLinux"),
        Some("redhat")
    );
}

#[test]
fn unknown_linux_gets_tux() {
    assert_eq!(parse("ID=homebrewdistro\nLinux"), Some("linux"));
}

#[test]
fn appliance_marker_beats_os_release() {
    let out = "ID=proxmox\nPRETTY_NAME=\"Debian GNU/Linux 12\"\nID=debian\nLinux";
    assert_eq!(parse(out), Some("proxmox"));
}

#[test]
fn kernel_name_without_os_release() {
    assert_eq!(parse("Darwin\n"), Some("macos"));
    assert_eq!(parse("OpenBSD\n"), Some("openbsd"));
}

#[test]
fn garbage_is_none() {
    assert_eq!(parse(""), None);
    assert_eq!(
        parse(
            "'cat' is not recognized as an internal or external command,\r\noperable program or batch file.\r\n"
        ),
        None
    );
    assert_eq!(parse("Windows_NT\n"), None);
    assert_eq!(parse("ID=../../etc/passwd\nSunOS"), None);
}

#[test]
fn every_slug_is_known() {
    assert!(slugs().all(is_known));
    assert!(!is_known("windows"));
}
