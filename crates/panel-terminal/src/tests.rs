use super::*;

#[test]
fn add_activates_new_tab() {
    let mut tabs = Tabs::default();
    assert!(tabs.is_empty());
    tabs.add(1);
    tabs.add(2);
    assert_eq!(tabs.active(), Some(2));
    assert_eq!(tabs.len(), 2);
}

#[test]
fn select_switches_and_ignores_unknown() {
    let mut tabs = Tabs::default();
    tabs.add(1);
    tabs.add(2);
    assert!(tabs.select(1));
    assert_eq!(tabs.active(), Some(1));
    assert!(!tabs.select(9));
    assert_eq!(tabs.active(), Some(1));
}

#[test]
fn removing_active_picks_right_neighbor_then_left() {
    let mut tabs = Tabs::default();
    tabs.add(1);
    tabs.add(2);
    tabs.add(3);
    tabs.select(2);
    assert_eq!(tabs.remove(2), Removal::Activated(3));
    assert_eq!(tabs.remove(3), Removal::Activated(1));
    assert_eq!(tabs.ids(), &[1]);
}

#[test]
fn removing_inactive_keeps_active() {
    let mut tabs = Tabs::default();
    tabs.add(1);
    tabs.add(2);
    assert_eq!(tabs.remove(1), Removal::Unchanged);
    assert_eq!(tabs.active(), Some(2));
}

#[test]
fn removing_last_reports_empty() {
    let mut tabs = Tabs::default();
    tabs.add(1);
    assert_eq!(tabs.remove(1), Removal::Empty);
    assert_eq!(tabs.active(), None);
    assert_eq!(tabs.remove(1), Removal::Unchanged);
}

#[test]
fn palette_colors_are_valid_hex() {
    let palette = palette();
    for value in [
        &palette.foreground,
        &palette.background,
        &palette.red,
        &palette.dim_white,
    ] {
        assert_eq!(value.len(), 7);
        assert!(value.starts_with('#'));
        assert!(u32::from_str_radix(&value[1..], 16).is_ok());
    }
}
