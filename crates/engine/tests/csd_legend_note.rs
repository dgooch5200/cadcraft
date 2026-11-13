//! The legend's power note wraps inside the legend, on both papers.

use cadcraft_csd::{MARGIN, Paper, Protocol, legend_block};
use cadcraft_doc::{EntityKind, Handle};

#[test]
fn power_note_stays_inside_the_legend() {
    for paper in [Paper::Letter, Paper::Tabloid] {
        let w = paper.size().0 - MARGIN * 2.0;
        let mut n = 0;
        let block = legend_block(paper, &[Protocol::Power], 1.0, || {
            n += 1;
            Handle(n)
        });
        let mut note_lines = 0;
        for e in block.entities.iter() {
            if let EntityKind::Text(t) = &e.kind {
                let right = t.insert.x + cadcraft_fonts::shape_stroke(&t.value, t.height, t.width_factor, t.oblique).width;
                assert!(right <= w, "{paper:?}: {:?} ends at {right}, legend is {w} wide", t.value);
                if t.value.contains("Power") || t.value.contains("calculations") || t.value.contains("professional") {
                    note_lines += 1;
                }
            }
        }
        assert!(note_lines >= 1);
        if paper == Paper::Letter {
            assert!(note_lines >= 2, "the note should wrap on letter");
        }
    }
}
