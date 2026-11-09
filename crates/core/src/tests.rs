//! Tests for the helpers in `lib.rs`.

use std::sync::{Arc, Mutex};

use super::*;

#[test]
fn locked_survives_a_poisoned_mutex() {
    let m = Arc::new(Mutex::new(1));
    let m2 = Arc::clone(&m);
    let _ = std::thread::spawn(move || {
        let _g = m2.lock().unwrap();
        panic!("poison it");
    })
    .join();
    assert!(m.is_poisoned());
    *locked(&m) += 1;
    assert_eq!(*locked(&m), 2);
}
