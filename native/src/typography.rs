//! CSS-style tracking for display headings and addon titles. GPUI's standard
//! text element doesn't expose letter spacing.
use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, LineLayout, Pixels, ShapedLine, SharedString, Style, TextAlign, TextRun,
    Window, WrapBoundary, WrappedLine, WrappedLineLayout, px,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub fn tracked(
    text: impl Into<SharedString>,
    font_size: f32,
    line_height: f32,
    spacing: f32,
) -> TrackedText {
    TrackedText {
        text: text.into(),
        font_size,
        line_height: font_size * line_height,
        spacing,
        wrap: false,
    }
}

pub fn wrapped(
    text: impl Into<SharedString>,
    font_size: f32,
    line_height: f32,
    spacing: f32,
) -> TrackedText {
    TrackedText {
        wrap: true,
        ..tracked(text, font_size, line_height, spacing)
    }
}

pub struct TrackedText {
    text: SharedString,
    font_size: f32,
    line_height: f32,
    spacing: f32,
    wrap: bool,
}

pub enum Layout {
    Single(ShapedLine),
    Wrapped(Rc<RefCell<Option<WrappedLine>>>),
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '\u{00c0}'..='\u{024f}' | '\u{0400}'..='\u{04ff}')
        || "-_.'$%@#^~,=:⋯".contains(c)
}

// Use the same word-boundary rules as GPUI, evaluated after tracking so wrapping
// and painting agree. Break only at shaped glyph boundaries (never UTF-8 bytes).
fn breaks(layout: &LineLayout, text: &str, limit: Pixels) -> Vec<WrapBoundary> {
    let glyphs: Vec<_> = layout
        .runs
        .iter()
        .enumerate()
        .flat_map(|(run_ix, run)| {
            run.glyphs.iter().enumerate().map(move |(glyph_ix, glyph)| {
                (
                    WrapBoundary { run_ix, glyph_ix },
                    text[glyph.index..].chars().next().unwrap(),
                    glyph.position.x,
                )
            })
        })
        .collect();
    wrap_at(&glyphs, layout.width, limit)
}

fn wrap_at(
    glyphs: &[(WrapBoundary, char, Pixels)],
    width: Pixels,
    limit: Pixels,
) -> Vec<WrapBoundary> {
    let mut breaks = Vec::new();
    let mut start = WrapBoundary {
        run_ix: 0,
        glyph_ix: 0,
    };
    let mut start_x = px(0.);
    let mut candidate = None;
    let mut seen_word = false;
    let mut previous = '\0';
    for (index, &(boundary, ch, x)) in glyphs.iter().enumerate() {
        if seen_word && ch != ' ' && (previous == ' ' || !word_char(ch)) {
            candidate = Some((boundary, x));
        }
        seen_word |= ch != ' ';
        let end_x = glyphs.get(index + 1).map_or(width, |glyph| glyph.2);
        if end_x - start_x > limit && boundary > start {
            let (next, next_x) = candidate.take().unwrap_or((boundary, x));
            start = next;
            start_x = next_x;
            breaks.push(next);
        }
        previous = ch;
    }
    breaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addon_titles_wrap_at_words_and_break_a_name_wider_than_the_column() {
        let wrap = |text: &str, width| {
            let glyphs: Vec<_> = text
                .chars()
                .enumerate()
                .map(|(index, ch)| {
                    (
                        WrapBoundary {
                            run_ix: 0,
                            glyph_ix: index,
                        },
                        ch,
                        px(index as f32 * 10.),
                    )
                })
                .collect();
            wrap_at(&glyphs, px(text.chars().count() as f32 * 10.), px(width))
                .into_iter()
                .map(|boundary| boundary.glyph_ix)
                .collect::<Vec<_>>()
        };
        assert_eq!(wrap("Mythic Dungeon Tools", 80.), vec![7, 15]);
        assert!(wrap("Mythic Dungeon Tools", 200.).is_empty());
        assert_eq!(wrap("ÉtoileÉtoile", 50.), vec![5, 10]);
        assert_eq!(wrap("ÉtoileÉtoile", 1.), (1..12).collect::<Vec<_>>());
    }
}

impl IntoElement for TrackedText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TrackedText {
    type RequestLayoutState = Layout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Layout) {
        let inherited = window.text_style();
        let run = TextRun {
            len: self.text.len(),
            font: inherited.font(),
            color: inherited.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut line = window.text_system().shape_line(
            self.text.clone(),
            px(self.font_size),
            &[run.clone()],
            None,
        );
        let mut runs = line.runs.clone();
        for run in &mut runs {
            for glyph in &mut run.glyphs {
                let preceding = self.text[..glyph.index].chars().count();
                glyph.position.x += px(self.spacing * preceding as f32);
            }
        }
        let width = line.width + px(self.spacing * self.text.chars().count() as f32);
        // Replace the shaped layout rather than mutating the text-system cache.
        *line = Arc::new(LineLayout {
            font_size: line.font_size,
            width,
            ascent: line.ascent,
            descent: line.descent,
            runs,
            len: line.len(),
        });
        if self.wrap {
            let unwrapped: Arc<LineLayout> = (*line).clone();
            let text = self.text.clone();
            let mut wrapped = window
                .text_system()
                .shape_text(text.clone(), px(self.font_size), &[run], None, None)
                .expect("addon title is valid text")
                .remove(0);
            let measured = Rc::new(RefCell::new(None));
            let state = measured.clone();
            let height = px(self.line_height);
            let id =
                window.request_measured_layout(Style::default(), move |known, available, _, _| {
                    let width = known.width.or(match available.width {
                        AvailableSpace::Definite(width) => Some(width.max(px(1.))),
                        _ => None,
                    });
                    *wrapped = Arc::new(WrappedLineLayout {
                        unwrapped_layout: unwrapped.clone(),
                        wrap_boundaries: width
                            .map_or_else(Vec::new, |width| breaks(&unwrapped, &text, width))
                            .into_iter()
                            .collect(),
                        wrap_width: width,
                    });
                    let size = wrapped.size(height);
                    state.replace(Some(wrapped.clone()));
                    size
                });
            return (id, Layout::Wrapped(measured));
        }
        let mut style = Style::default();
        style.size.width = width.max(px(0.)).into();
        style.size.height = px(self.line_height).into();
        (window.request_layout(style, [], cx), Layout::Single(line))
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Layout,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        line: &mut Layout,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        match line {
            Layout::Single(line) => {
                let _ = line.paint(bounds.origin, px(self.line_height), window, cx);
            }
            Layout::Wrapped(state) => {
                if let Some(line) = state.borrow().as_ref() {
                    let _ = line.paint(
                        bounds.origin,
                        px(self.line_height),
                        TextAlign::Left,
                        Some(bounds),
                        window,
                        cx,
                    );
                }
            }
        }
    }
}
