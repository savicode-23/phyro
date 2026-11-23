use super::*;
use std::time::Instant;

/// Apply the same three selected curves with unrelated off-artboard rectangles
/// on either side of and between them. Drawing setup and cloning are untimed.
#[test]
#[ignore = "release timing benchmark"]
fn sparse_curved_boolean_edits() {
    for count in [0, 1_000, 10_000] {
        let mut drawing = Document::default();
        let back = drawing.add_shape(
            [16., 26.],
            [72., 55.],
            [35, 98, 128, 211],
            Shape::Ellipse,
            1,
        );
        drawing.objects[0].rotation_keys.insert(2, -23.);
        let middle = drawing.add_shape([45., 25.], [57., 63.], [130; 4], Shape::Ellipse, 1);
        drawing.objects[1].animated.skew = -12.;
        let front = drawing.add_shape(
            [38., 34.],
            [57., 48.],
            [211; 4],
            Shape::Path {
                closed: true,
                nodes: vec![
                    PathNode {
                        handle_style: Default::default(),
                        point: [0., 0.25],
                        incoming: [-0.1, 0.55],
                        outgoing: [0.2, -0.12],
                    },
                    PathNode {
                        handle_style: Default::default(),
                        point: [1., 0.24],
                        incoming: [0.74, 0.1],
                        outgoing: [1., 0.51],
                    },
                    PathNode {
                        handle_style: Default::default(),
                        point: [0.38, 1.],
                        incoming: [0.65, 1.07],
                        outgoing: [0.24, 0.85],
                    },
                ],
            },
            1,
        );
        drawing.objects[2].rotation = 13.;
        let mut operands = std::mem::take(&mut drawing.objects).into_iter();
        let template = {
            let mut empty = Document::default();
            empty.add_rectangle([512.; 2], [3.; 2], [255; 4]);
            empty.objects.pop().unwrap()
        };
        for index in 0..=count {
            for slot in [count / 4, count / 2, count * 3 / 4] {
                if index == slot {
                    drawing.objects.push(operands.next().unwrap());
                }
            }
            if index < count {
                let mut other = template.clone();
                other.id = drawing.next_id;
                drawing.next_id += 1;
                drawing.objects.push(other);
            }
        }
        for op in [
            BooleanOp::Difference,
            BooleanOp::Union,
            BooleanOp::Intersection,
        ] {
            let mut times = Vec::new();
            for sample in 0..33 {
                let mut current = drawing.clone();
                let selected = [front, back, middle];
                let start = Instant::now();
                let result = std::hint::black_box(&mut current)
                    .boolean(&selected, 2, op)
                    .unwrap();
                let elapsed = start.elapsed().as_secs_f64() * 1_000.;
                assert_eq!(result, [back]);
                assert_eq!(current.objects.len(), count + 1);
                if sample > 0 {
                    times.push(elapsed);
                }
            }
            times.sort_by(f64::total_cmp);
            println!(
                "{count} unrelated / {op:?}: p50 {:.3} ms, p95 {:.3} ms",
                times[15], times[30]
            );
        }
    }
}
