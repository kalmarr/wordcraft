//! First-use regression in a fresh process, including two windows caching unchecked layouts.
use wordcraft_doc::Document;
use wordcraft_engine::{Session, proof::hu};
use wordcraft_ui_egui::WordApp;

fn tick(app: &mut WordApp, ctx: &egui::Context) {
    ctx.run_ui(Default::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
}
fn spelling_count(app: &mut WordApp) -> usize {
    app.session
        .layout()
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .map(
            |item| {
                if let wordcraft_layout::Placed::Lines { para, .. } = item {
                    para.issues.iter().filter(|(_, _, grammar)| !grammar).count()
                } else {
                    0
                }
            },
        )
        .sum()
}

#[test]
fn completed_loading_rechecks_every_open_window_without_editing() {
    let mut doc = Document::from_text("hejesírás és árvíztűrő.");
    doc.styles.default_chr.lang = Some("hu-HU".into());
    let mut first = WordApp::new(Session::new(doc.clone()), Default::default());
    let mut second = WordApp::new(Session::new(doc), Default::default());
    first.ui.language = "hu".into();
    second.ui.language = "hu".into();
    let a = egui::Context::default();
    let b = egui::Context::default();
    tick(&mut first, &a);
    tick(&mut second, &b);
    assert_eq!(spelling_count(&mut first), 0);
    assert_eq!(spelling_count(&mut second), 0);
    assert!(hu::is_loading());
    for _ in 0..1000 {
        tick(&mut first, &a);
        if !hu::is_loading() {
            break;
        }
    }
    assert!(!hu::is_loading());
    assert_eq!(spelling_count(&mut first), 1);
    // Completion happened in the other window; this one still has to invalidate its own cache.
    tick(&mut second, &b);
    assert_eq!(spelling_count(&mut second), 1);
}
