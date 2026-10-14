use super::*;

#[test]
fn sealed_bytes_hide_the_plain_text() {
    let s = Secret::new("hunter2-very-secret").unwrap();
    let needle = b"hunter2-very-secret";
    assert!(!s.sealed.windows(needle.len()).any(|w| w == needle));
    assert!(!format!("{s:?}").contains("hunter2"));
    assert_eq!(*s.reveal(), "hunter2-very-secret");
}

#[test]
fn same_secret_seals_differently() {
    let (a, b) = (Secret::new("pw").unwrap(), Secret::new("pw").unwrap());
    assert_ne!(a.sealed, b.sealed);
    assert_eq!(*Secret::new("").unwrap().reveal(), "");
}
