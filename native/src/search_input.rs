use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, GlobalElementId, InspectorElementId, LayoutId,
    MouseButton, Pixels, Point, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
    div, fill, point, prelude::*, px, relative, rgba, size,
};
use std::ops::Range;

pub struct SearchInput {
    pub focus: FocusHandle,
    text: SharedString,
    anchor: usize,
    caret: usize,
    marked: Option<Range<usize>>,
    layout: Option<(ShapedLine, Bounds<Pixels>, Pixels)>,
    dragging: bool,
    limit: Option<usize>,
}

pub struct Changed(pub String);
impl gpui::EventEmitter<Changed> for SearchInput {}

impl SearchInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            text: "".into(),
            anchor: 0,
            caret: 0,
            marked: None,
            layout: None,
            dragging: false,
            limit: None,
        }
    }

    pub fn set_limit(&mut self, units: usize) {
        self.limit = Some(units);
    }

    pub fn set(&mut self, value: &str, cx: &mut Context<Self>) {
        self.text = value.to_owned().into();
        self.anchor = value.len();
        self.caret = value.len();
        self.marked = None;
        self.layout = None;
        cx.notify();
    }

    fn selected(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn select(&mut self, offset: usize, extend: bool, cx: &mut Context<Self>) {
        self.caret = offset;
        if !extend {
            self.anchor = offset;
        }
        cx.notify();
    }

    fn previous(&self) -> usize {
        self.text[..self.caret]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next(&self) -> usize {
        self.text[self.caret..]
            .chars()
            .next()
            .map_or(self.caret, |ch| self.caret + ch.len_utf8())
    }

    fn hit(&self, position: Point<Pixels>) -> usize {
        self.layout
            .as_ref()
            .filter(|(line, _, _)| line.text == self.text)
            .map_or(self.text.len(), |(line, bounds, scroll)| {
                line.closest_index_for_x(position.x - bounds.left() + *scroll)
                    .min(self.text.len())
            })
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let mut value = self.text.to_string();
        let available = self.limit.map(|limit| {
            limit.saturating_sub(
                self.text[..range.start].encode_utf16().count()
                    + self.text[range.end..].encode_utf16().count(),
            )
        });
        let text = available.map_or(text, |units| &text[..utf16_prefix(text, units)]);
        value.replace_range(range.clone(), text);
        self.text = value.into();
        self.anchor = range.start + text.len();
        self.caret = self.anchor;
        self.marked = None;
        cx.emit(Changed(self.text.to_string()));
        cx.notify();
    }

    fn range_for_edit(&self, utf16: Option<Range<usize>>) -> Range<usize> {
        utf16
            .map(|range| self.byte_range(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selected())
    }

    fn byte_range(&self, range: Range<usize>) -> Range<usize> {
        byte_offset(&self.text, range.start)..byte_offset(&self.text, range.end)
    }

    fn utf16_range(&self, range: Range<usize>) -> Range<usize> {
        self.text[..range.start].encode_utf16().count()
            ..self.text[..range.end].encode_utf16().count()
    }

    fn edit_key(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let modifiers = key.modifiers;
        let shortcut = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        let selection = self.selected();
        match (shortcut, key.key.as_str()) {
            (true, "a") => {
                self.anchor = 0;
                self.select(self.text.len(), true, cx);
            }
            (true, "c" | "x") if !selection.is_empty() => {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    self.text[selection.clone()].into(),
                ));
                if key.key == "x" {
                    self.replace(selection, "", cx);
                }
            }
            (true, "v") => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(selection, &single_line(&text), cx);
                }
            }
            (false, "left") => {
                let offset = if !modifiers.shift && !selection.is_empty() {
                    selection.start
                } else {
                    self.previous()
                };
                self.select(offset, modifiers.shift, cx);
            }
            (false, "right") => {
                let offset = if !modifiers.shift && !selection.is_empty() {
                    selection.end
                } else {
                    self.next()
                };
                self.select(offset, modifiers.shift, cx);
            }
            (false, "home") | (true, "left") if !modifiers.alt => {
                self.select(0, modifiers.shift, cx)
            }
            (false, "end") | (true, "right") if !modifiers.alt => {
                self.select(self.text.len(), modifiers.shift, cx)
            }
            (false, "backspace" | "delete") if self.marked.is_none() => {
                let range = if !selection.is_empty() {
                    selection
                } else if key.key == "backspace" {
                    self.previous()..self.caret
                } else {
                    self.caret..self.next()
                };
                self.replace(range, "", cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

fn utf16_prefix(text: &str, units: usize) -> usize {
    let mut remaining = units;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        if ch.len_utf16() > remaining {
            break;
        }
        remaining -= ch.len_utf16();
        end = index + ch.len_utf8();
    }
    end
}

fn byte_offset(text: &str, units: usize) -> usize {
    let mut count = 0;
    for (index, ch) in text.char_indices() {
        if count >= units {
            return index;
        }
        count += ch.len_utf16();
    }
    text.len()
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if matches!(ch, '\r' | '\n' | '\t') {
                ' '
            } else {
                ch
            }
        })
        .filter(|ch| !ch.is_control())
        .collect()
}

impl EntityInputHandler for SearchInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.byte_range(range);
        *actual = Some(self.utf16_range(range.clone()));
        Some(self.text[range].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf16_range(self.selected()),
            reversed: self.caret < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|range| self.utf16_range(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(self.range_for_edit(range), &single_line(text), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.range_for_edit(range);
        let start = range.start;
        let text = single_line(text);
        self.replace(range, &text, cx);
        self.marked = (!text.is_empty()).then_some(start..start + text.len());
        if let Some(selected) = selected {
            self.anchor = start + byte_offset(&text, selected.start);
            self.caret = start + byte_offset(&text, selected.end);
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (line, bounds, scroll) = self.layout.as_ref()?;
        if line.text != self.text && !self.text.is_empty() {
            return None;
        }
        let range = self.byte_range(range);
        let left = bounds.left() - *scroll;
        Some(Bounds::from_corners(
            point(left + line.x_for_index(range.start), bounds.top()),
            point(left + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.layout.as_ref()?;
        Some(self.text[..self.hit(position)].encode_utf16().count())
    }
}

pub fn field(
    input: &Entity<SearchInput>,
    placeholder: &'static str,
    muted: u32,
    cx: &mut App,
) -> impl IntoElement {
    input.update(cx, |input, cx| {
        div()
            .track_focus(&input.focus)
            .flex_1()
            .min_w_0()
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(SearchInput::edit_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|input, event: &gpui::MouseDownEvent, window, cx| {
                    input.focus.focus(window);
                    input.dragging = true;
                    input.select(input.hit(event.position), event.modifiers.shift, cx);
                }),
            )
            .on_mouse_move(cx.listener(|input, event: &gpui::MouseMoveEvent, _, cx| {
                if input.dragging {
                    input.select(input.hit(event.position), true, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|input, _, _, _| input.dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|input, _, _, _| input.dragging = false),
            )
            .child(Line {
                input: cx.entity(),
                placeholder,
                muted,
            })
    })
}

struct Line {
    input: Entity<SearchInput>,
    placeholder: &'static str,
    muted: u32,
}

impl IntoElement for Line {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Line {
    type RequestLayoutState = ();
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
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(18.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = self.input.read(cx);
        let focus = input.focus.clone();
        let focused = focus.is_focused(window);
        let empty = input.text.is_empty();
        let text = if empty {
            self.placeholder.into()
        } else {
            input.text.clone()
        };
        let style = window.text_style();
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: if empty {
                gpui::rgb(self.muted).into()
            } else {
                style.color
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(text, px(12.), &[run], None);
        let caret = line.x_for_index(input.caret);
        let selected = input.selected();
        let marked = input.marked.clone();
        let prior_scroll = input
            .layout
            .as_ref()
            .map_or(px(0.), |(_, _, scroll)| *scroll);
        let visible = (bounds.size.width - px(2.)).max(px(0.));
        let scroll = prior_scroll
            .min(caret)
            .min((line.width - visible).max(px(0.)))
            .max(caret - visible);
        let origin = point(bounds.left() - scroll, bounds.top());
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focused && !selected.is_empty() {
                let start = line.x_for_index(selected.start);
                let end = line.x_for_index(selected.end);
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + start.min(end), origin.y),
                        size((end - start).abs(), bounds.size.height),
                    ),
                    rgba(0x527dff50),
                ));
            }
            let _ = line.paint(origin, bounds.size.height, window, cx);
            if focused && selected.is_empty() {
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + caret, origin.y + px(2.)),
                        size(px(1.), px(14.)),
                    ),
                    style.color,
                ));
            }
            if let Some(marked) = marked {
                let start = line.x_for_index(marked.start);
                let end = line.x_for_index(marked.end);
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + start.min(end), bounds.bottom() - px(2.)),
                        size((end - start).abs(), px(1.)),
                    ),
                    style.color,
                ));
            }
        });
        self.input
            .update(cx, |input, _| input.layout = Some((line, bounds, scroll)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_limit_preserves_whole_unicode_characters() {
        assert_eq!(&"a🐻é"[..utf16_prefix("a🐻é", 2)], "a");
        assert_eq!(&"a🐻é"[..utf16_prefix("a🐻é", 3)], "a🐻");
        assert_eq!(utf16_prefix("a🐻é", 0), 0);
        assert_eq!(utf16_prefix("a🐻é", 99), "a🐻é".len());
    }

    #[test]
    fn native_utf16_ranges_map_to_utf8_boundaries() {
        let text = "a🐻é";
        assert_eq!(byte_offset(text, 1), 1);
        assert_eq!(byte_offset(text, 3), 5);
        assert_eq!(byte_offset(text, 4), text.len());
        assert_eq!(byte_offset(text, 99), text.len());
        assert_eq!(single_line("Big\nWigs\u{7}"), "Big Wigs");
    }
}
