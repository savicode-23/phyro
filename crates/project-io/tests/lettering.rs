use document_core::typography::{TextAlign, TextStyle, Typeface};
use document_core::{AnimationProject, Document};
use project_io::{export_svg, import_svg, load_animation, save_animation};

#[test]
fn inkscape_single_line_span_uses_its_position_and_inherited_styling() {
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("text.svg");
    let import = |body: &str| {
        std::fs::write(&svg, format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" width="120" height="48">{body}</svg>"#)).unwrap();
        import_svg(&svg)
    };
    let owned = import(r##"<text x="1" y="2" font-family="Hack" font-size="12" fill="#34599b" opacity=".8"><tspan sodipodi:role="line" x="12" y="30" style="font-size:18px;opacity:.5;writing-mode:lr-tb;direction:ltr;font-variant:normal;font-stretch:normal">120</tspan></text>"##).unwrap();
    let flat = import(r##"<text x="12" y="30" font-family="Hack" font-size="18" fill="#34599b" opacity=".4">120</text>"##).unwrap();
    assert_eq!(owned.objects[0], flat.objects[0]);
    assert_eq!(owned.objects[0].text.as_ref().unwrap().content, "120");
    for body in [
        "<text font-family='Hack'>A<tspan>B</tspan></text>",
        "<text font-family='Hack'><tspan>A</tspan><tspan>B</tspan></text>",
    ] {
        assert!(
            import(body).is_err(),
            "mixed lettering must retain its layout"
        );
    }
}

#[test]
fn native_lettering_stays_editable_and_outlined_svg_matches_the_preview() {
    let mut doc = Document::default();
    let lettering = TextStyle {
        content: "PÖLY\n18".into(),
        face: Typeface::HackRegular,
        size: 22.,
        align: TextAlign::Center,
        letter_spacing: 1.5,
        ..Default::default()
    };
    lettering
        .clone()
        .insert(&mut doc, 1, [14., 19.], [34, 190, 120, 255])
        .unwrap();
    let object = &mut doc.objects[0];
    object.rotation = 13.;
    object.stroke = [24, 39, 60, 255];
    object.stroke_width = 0.75;
    let image = renderer::render(&doc, 0).unwrap();
    assert!(image.pixels().iter().filter(|p| p.alpha() > 0).count() > 140);
    let project = AnimationProject::from_document(doc);
    let tmp = tempfile::tempdir().unwrap();
    let native = tmp.path().join("lettering.vanim");
    save_animation(&native, &project, false).unwrap();
    let reopened = load_animation(&native).unwrap();
    assert_eq!(
        reopened.clips[0].drawing_at(0).objects[0].text,
        Some(lettering)
    );
    let path = tmp.path().join("outlined.svg");
    export_svg(&path, reopened.clips[0].drawing_at(0), 0, false).unwrap();
    let imported = import_svg(&path).unwrap();
    let rerender = renderer::render(&imported, 0).unwrap();
    let sum: u64 = image
        .data()
        .iter()
        .zip(rerender.data())
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    assert!(
        sum < image.data().len() as u64 / 5,
        "Mean channel error was {}",
        sum as f32 / image.data().len() as f32
    );
}
