use super::*;

/// A localized update to an existing rendering at the same frame and density.
pub enum EditPatch {
    FullRender,
    Unchanged,
    Pixels { origin: [usize; 2], pixels: Pixmap },
}

fn structural_changes<'a>(
    old: &'a [VectorObject],
    new: &'a [VectorObject],
) -> Option<Vec<&'a VectorObject>> {
    // Skip unchanged outer spans before indexing. A local cut commonly
    // replaces a few adjacent paths in a document with a long untouched tail.
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let (old, new) = (&old[prefix..], &new[prefix..]);
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (old, new) = (&old[..old.len() - suffix], &new[..new.len() - suffix]);
    // Insertions and removals shift slots without moving their pixels. Compare
    // the surviving IDs and let a full render handle any simultaneous reorder.
    let mut remaining: std::collections::HashMap<_, _> = old
        .iter()
        .enumerate()
        .map(|(index, o)| (o.id, (index, o)))
        .collect();
    if remaining.len() != old.len() {
        return None;
    }
    let mut changed = Vec::new();
    let mut last = None;
    for object in new {
        if let Some((index, previous)) = remaining.remove(&object.id) {
            if last.is_some_and(|last| index <= last) {
                return None;
            }
            last = Some(index);
            if previous != object {
                changed.extend([previous, object]);
            }
        } else {
            changed.push(object);
        }
    }
    changed.extend(remaining.into_values().map(|(_, o)| o));
    Some(changed)
}

/// Compare successive authored scenes drawn at one frame. The caller retains
/// the previous image and replaces only the returned pixels.
pub fn edit_patch(
    before: &Document,
    after: &Document,
    frame: u32,
    scale: f32,
) -> Result<EditPatch, String> {
    if [before.width, before.height] != [after.width, after.height]
        || before.layers != after.layers
        || before.skeleton != after.skeleton
    {
        return Ok(EditPatch::FullRender);
    }
    let page = artboard(after, frame, scale)?;
    let bounds = |o: &VectorObject| visible_bounds(o, frame, &page);
    let mut dirty: Option<IntRect> = None;
    let mut mark_dirty = |object: &VectorObject| {
        if !after
            .layers
            .iter()
            .any(|l| l.id == object.layer && l.visible)
        {
            return;
        }
        if let Some(rect) = bounds(object) {
            dirty = Some(dirty.map_or(rect, |a| enclosing(a, rect)));
        }
    };

    // Moving or resizing objects keeps their slots. Scan forwards once without
    // allocating, and check identity only for pairs that actually differ.
    let mut same_slots = before.objects.len() == after.objects.len();
    let mut check_runs = !same_slots;
    if same_slots {
        for (old, new) in before
            .objects
            .iter()
            .zip(&after.objects)
            .filter(|(a, b)| a != b)
        {
            if old.id != new.id {
                same_slots = false;
                check_runs = true;
                break;
            }
            check_runs |=
                old.layer != new.layer || old.clip_source.is_some() || new.clip_source.is_some();
            mark_dirty(old);
            mark_dirty(new);
        }
    }
    if !same_slots {
        let Some(changed) = structural_changes(&before.objects, &after.objects) else {
            return Ok(EditPatch::FullRender);
        };
        changed.into_iter().for_each(mark_dirty);
    }
    // Moving an intervening object to another layer, for example, can join two
    // clipped runs elsewhere even when that object itself is off the artboard.
    if check_runs
        && crate::clip_groups::shared_slots(before, frame)
            != crate::clip_groups::shared_slots(after, frame)
    {
        return Ok(EditPatch::FullRender);
    }
    let Some(area) = dirty else {
        return Ok(EditPatch::Unchanged);
    };
    // Widely separated edits can cover most of the surface. Use the regular
    // drag compositor in that case so its cached surroundings serve later frames.
    if u64::from(area.width()) * u64::from(area.height()) * 3
        > u64::from(page.bounds.width()) * u64::from(page.bounds.height())
    {
        return Ok(EditPatch::FullRender);
    }
    // Render in original coordinates so antialiasing, gradients and masks
    // match the full artboard, including at the edges of the uploaded patch.
    let mut surface = page.allocate()?;
    let ordered = after.paint_order();
    let objects = crate::clip_groups::runs(ordered.into_iter(), frame)
        .filter(|(_, o)| bounds(o).is_some_and(|r| r.intersect(&area).is_some()));
    let mut symbols = crate::bounds::SymbolBounds::new();
    crate::clip_groups::paint_with_layers(
        &mut surface,
        objects,
        &after.layers,
        (frame, page.transform, [0.; 2]),
        Some(&mut symbols),
        crate::RenderPurpose::Editor,
    )?;
    let pixels = surface
        .clone_rect(area)
        .ok_or("Could not allocate the preview region.")?;
    Ok(EditPatch::Pixels {
        origin: [area.left() as usize, area.top() as usize],
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_core::{Shape, Stencil};

    fn replace_pixels(image: &mut Pixmap, [x, y]: [usize; 2], pixels: &Pixmap) {
        let stride = image.width() as usize;
        for (dy, row) in pixels
            .pixels()
            .chunks_exact(pixels.width() as usize)
            .enumerate()
        {
            let start = (y + dy) * stride + x;
            image.pixels_mut()[start..start + row.len()].copy_from_slice(row);
        }
    }

    #[test]
    fn parked_edits_skip_page_pixels_until_the_stroke_reaches_the_edge() {
        let mut original = Document::default();
        original.add_shape([-30., 16.], [10., 21.], [0; 4], Shape::Ellipse, 1);
        original.objects[0].stroke_width = 4.;
        original.objects[0].stroke = [42, 149, 87, 255];
        let mut parked = original.clone();
        parked.objects[0].position[0] = -21.;
        let scale = 2.;
        assert!(matches!(
            edit_patch(&original, &parked, 0, scale).unwrap(),
            EditPatch::Unchanged
        ));
        assert!(edit_patch(&original, &parked, original.frames, scale).is_err());
        assert!(edit_patch(&original, &parked, 0, f32::INFINITY).is_err());

        let mut crossing = parked.clone();
        crossing.objects[0].position[0] = -10.5;
        let mut composite = crate::render_scaled(&original, 0, scale).unwrap();
        for (from, to) in [(&parked, &crossing), (&crossing, &parked)] {
            let EditPatch::Pixels { origin, pixels } = edit_patch(from, to, 0, scale).unwrap()
            else {
                panic!("Expected an update at the page edge");
            };
            assert_eq!(origin[0], 0);
            replace_pixels(&mut composite, origin, &pixels);
            let reference = crate::render_scaled(to, 0, scale).unwrap();
            assert_eq!(composite.data(), reference.data());
            assert_eq!(
                composite.pixels().iter().any(|p| p.alpha() > 0),
                to == &crossing
            );
        }
    }

    #[test]
    fn localized_changes_exactly_restore_old_and_new_pixels() {
        let mut before = Document::default();
        before.add_rectangle([0.; 2], [128.; 2], [37, 71, 83, 191]);
        before.add_shape(
            [8.2, 19.],
            [16., 23.],
            [35, 80, 178, 163],
            Shape::Ellipse,
            1,
        );
        before.add_shape(
            [20., 16.],
            [25., 33.],
            [80, 167, 102, 219],
            Shape::Rectangle,
            1,
        );
        let top = &mut before.objects[2];
        top.stroke = [43, 120, 171, 192];
        top.stroke_width = 2.4;
        top.opacity = 0.71;
        top.stencil = Some(Stencil {
            shape: Shape::Ellipse,
            origin: [0.05; 2],
            size: [0.9; 2],
            fill_rule: Default::default(),
            alpha: Some([123, 212]),
        });
        let scale = 1.73;
        let mut image = crate::render_scaled(&before, 0, scale).unwrap();
        for (position, angle, width) in [
            ([12.3, 22.], 12.2, 19.),
            ([26.6, 20.7], -32., 12.),
            ([-6., 20.1], 0., 14.),
        ] {
            let mut after = before.clone();
            let shape = &mut after.objects[1];
            shape.position = position;
            shape.rotation = angle;
            shape.size[0] = width;
            let EditPatch::Pixels {
                origin: [x, y],
                pixels,
            } = edit_patch(&before, &after, 0, scale).unwrap()
            else {
                panic!("local geometry change should yield a patch");
            };
            replace_pixels(&mut image, [x, y], &pixels);
            assert!(image.data() == crate::render_scaled(&after, 0, scale).unwrap().data());
            before = after;
        }
        let mut after = before.clone();
        after.layers[0].visible = false;
        assert!(matches!(
            edit_patch(&before, &after, 0, scale).unwrap(),
            EditPatch::FullRender
        ));
        after = before.clone();
        after.objects[0].fill = [100; 4];
        assert!(matches!(
            edit_patch(&before, &after, 0, scale).unwrap(),
            EditPatch::FullRender
        ));
    }

    #[test]
    fn adding_and_removing_nonadjacent_operands_preserves_unchanged_layers() {
        use document_core::{BooleanOp, Layer};
        let mut original = Document::default();
        let base = original.add_rectangle([19., 27.], [36., 41.], [191, 81, 67, 215]);
        // This translucent painted surface lies between the operands in storage.
        // It should be included in the composition, but not expand its damaged area.
        original.add_rectangle([0.; 2], [128.; 2], [34, 135, 170, 92]);
        let cutter = original.add_shape([31., 37.], [21., 17.], [180; 4], Shape::Ellipse, 1);
        original.add_rectangle([98., 83.], [12., 19.], [90; 4]);
        original.layers.push(Layer {
            id: 2,
            name: "Hidden".into(),
            visible: false,
            locked: false,
            blend_mode: Default::default(),
        });
        let mut cut = original.clone();
        cut.boolean(&[base, cutter], 0, BooleanOp::Difference)
            .unwrap();
        // Simultaneously insert large hidden artwork; only the visible cut can alter pixels.
        cut.add_shape([0.; 2], [128.; 2], [255; 4], Shape::Rectangle, 2);
        cut.add_shape([0.; 2], [128.; 2], [255; 4], Shape::Ellipse, 2);
        let mut same_count = cut.clone();
        same_count.objects.pop();
        assert_eq!(original.objects.len(), same_count.objects.len());
        let scale = 1.61;
        let mut image = crate::render_scaled(&original, 0, scale).unwrap();
        for (from, to) in [
            (&original, &cut),
            (&cut, &original),
            (&original, &same_count),
        ] {
            let EditPatch::Pixels { origin, pixels } = edit_patch(from, to, 0, scale).unwrap()
            else {
                panic!("expected a localized structural edit")
            };
            assert!(pixels.width() < image.width() / 2 && pixels.height() < image.height() / 2);
            replace_pixels(&mut image, origin, &pixels);
            assert!(image.data() == crate::render_scaled(to, 0, scale).unwrap().data());
        }
        let mut hidden = cut.clone();
        hidden.objects.pop();
        assert!(matches!(
            edit_patch(&cut, &hidden, 0, scale).unwrap(),
            EditPatch::Unchanged
        ));
        let mut reordered = cut;
        reordered.objects.swap(0, 1);
        assert!(matches!(
            edit_patch(&original, &reordered, 0, scale).unwrap(),
            EditPatch::FullRender
        ));
    }
}
