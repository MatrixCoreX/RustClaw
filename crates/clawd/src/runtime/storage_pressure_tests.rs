use super::*;
#[test]
fn critical_reserve_preserves_control_plane_space_and_unknown_probes_fail_closed() {
    assert_eq!(
        volume("fixture", Some(GIB / 4), Some(100 * GIB)).state,
        "critical"
    );
    assert_eq!(
        volume("fixture", Some(2 * GIB), Some(100 * GIB)).state,
        "warning"
    );
    assert_eq!(
        volume("fixture", Some(10 * GIB), Some(100 * GIB)).state,
        "normal"
    );
    assert_eq!(
        volume("fixture", None, Some(100 * GIB)).state,
        "unavailable"
    );
    assert_eq!(volume("fixture", Some(0), Some(0)).state, "unavailable");
}
#[test]
fn small_disks_keep_absolute_reserve_and_large_disks_scale_it() {
    assert!(volume("fixture", Some(GIB), Some(10 * GIB)).critical_reserve_bytes >= GIB / 2);
    assert!(volume("fixture", Some(10 * GIB), Some(1000 * GIB)).critical_reserve_bytes >= 10 * GIB);
}
