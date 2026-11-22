use super::*;

fn settle(s: &mut Spring) -> usize {
    (1..=600)
        .find(|_| !s.step(1. / 60.))
        .expect("spring settles")
}

#[test]
fn spring_settles_exactly_on_its_target() {
    let mut s = Spring::new(0.);
    s.target = 0.5;
    let frames = settle(&mut s);
    assert!(frames < 60, "{frames}");
    assert_eq!(s, Spring::new(0.5));
}

#[test]
fn retargeting_keeps_the_speed() {
    let mut s = Spring::new(0.);
    s.target = 1.;
    (0..5).for_each(|_| {
        s.step(1. / 60.);
    });
    let (value, velocity) = (s.value, s.velocity);
    assert!(velocity > 0.);
    s.target = 0.;
    assert_eq!((s.value, s.velocity), (value, velocity));
    let mut still = Spring::new(value);
    still.target = 0.;
    s.step(1. / 60.);
    still.step(1. / 60.);
    assert!(s.value > still.value, "{s:?} {still:?}");
    settle(&mut s);
    assert_eq!(s, Spring::new(0.));
}

#[test]
fn a_settled_spring_does_not_ask_for_frames() {
    assert!(!Spring::new(0.25).step(1. / 60.));
}
