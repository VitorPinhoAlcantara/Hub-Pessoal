use hub_ui_kit::Section;

use super::*;

#[test]
fn section_measures_after_first_tick_and_renders() {
    let mut section = Performance::new();
    assert_eq!(section.title(), "Desempenho");
    assert!(!section.ready());
    let _ = section.view();
    let _ = section.update(Message::Tick);
    assert!(section.ready());
    assert!(section.snapshot().unwrap().ram_total > 0);
    assert_eq!(section.cpu_history().len(), 1);
    assert_eq!(section.gpu_history().len(), 1);
    assert_eq!(section.ram_history().len(), 1);
    let _ = section.view();
}
