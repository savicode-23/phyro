use super::{clip_groups, Pixmap, Transform, VectorObject};

pub(super) fn draw_symbol_content(
    target: &mut Pixmap,
    object: &VectorObject,
    frame: u32,
    transform: Transform,
    page_origin: [f32; 2],
    purpose: super::RenderPurpose,
) -> Result<(), String> {
    let (definition, local_frame) = object
        .instance_at(frame)
        .ok_or("Missing instance source.")?;
    let local = object
        .symbol_transform(frame)
        .ok_or("Invalid symbol transform.")?
        .post_concat(transform);
    clip_groups::paint(
        target,
        clip_groups::runs(definition.paint_order().into_iter(), local_frame),
        local_frame,
        local,
        page_origin,
        None,
        purpose,
    )
}
