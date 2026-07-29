use super::HostPlatform;

#[test]
fn current_platform_carries_the_exact_cargo_target() {
    let current = HostPlatform::current();
    let target = current.target.as_deref().expect("compiled target triple");
    let parsed = HostPlatform::from_target(target).expect("supported compiled target");

    assert_eq!(parsed.os, current.os);
    assert_eq!(parsed.arch, current.arch);
    assert_eq!(parsed.target, current.target);
}
