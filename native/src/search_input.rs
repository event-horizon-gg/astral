use crate::text_history::{History, Kind, Snapshot};
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
    history: History,
    blur: Option<gpui::Subscription>,
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
            history: History::default(),
            blur: None,
        }
    }

    pub fn set_limit(&mut self, units: usize) {
        self.limit = Some(units);
    }

    pub fn set(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.text.as_ref() == value {
            return;
        }
        self.text = value.to_owned().into();
        self.anchor = value.len();
        self.caret = value.len();
        self.marked = None;
        self.layout = None;
        self.history.clear();
        cx.notify();
    }

    fn selected(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn select(&mut self, offset: usize, extend: bool, cx: &mut Context<Self>) {
        self.history.close_group();
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

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            anchor: self.anchor,
            caret: self.caret,
        }
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        let changed = self.text != snapshot.text;
        self.text = snapshot.text;
        self.anchor = snapshot.anchor;
        self.caret = snapshot.caret;
        self.marked = None;
        self.layout = None;
        if changed {
            cx.emit(Changed(self.text.to_string()));
        }
        cx.notify();
    }

    fn finish_composition(&mut self, cx: &mut Context<Self>) {
        self.marked = None;
        self.history.commit_composition(self.snapshot());
        self.history.close_group();
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, text: &str, kind: Kind, cx: &mut Context<Self>) {
        let before = self.snapshot();
        self.replace_raw(range, text, cx);
        self.history.record(before, self.snapshot(), kind);
    }

    fn replace_raw(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let (value, inserted) = replacement(&self.text, range, text, self.limit);
        let changed = self.text.as_ref() != value;
        self.text = value.into();
        self.anchor = inserted.end;
        self.caret = self.anchor;
        self.marked = None;
        self.layout = None;
        if changed {
            cx.emit(Changed(self.text.to_string()));
        }
        cx.notify();
        inserted
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
        if let Some(action) = history_action(key, cfg!(target_os = "macos")) {
            if !self.history.composing() {
                let snapshot = match action {
                    HistoryAction::Undo => self.history.undo(),
                    HistoryAction::Redo => self.history.redo(),
                };
                if let Some(snapshot) = snapshot {
                    self.restore(snapshot, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
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
                    self.replace(selection, "", Kind::Separate, cx);
                }
            }
            (true, "v") => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(selection, &single_line(&text), Kind::Separate, cx);
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
                self.replace(
                    range,
                    "",
                    if key.key == "backspace" {
                        Kind::Backspace
                    } else {
                        Kind::ForwardDelete
                    },
                    cx,
                );
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

#[derive(Debug, PartialEq)]
enum HistoryAction {
    Undo,
    Redo,
}

fn replacement(
    value: &str,
    range: Range<usize>,
    text: &str,
    limit: Option<usize>,
) -> (String, Range<usize>) {
    let available = limit.map(|limit| {
        limit.saturating_sub(
            value[..range.start].encode_utf16().count() + value[range.end..].encode_utf16().count(),
        )
    });
    let accepted = available.map_or(text, |units| &text[..utf16_prefix(text, units)]);
    let inserted = range.start..range.start + accepted.len();
    let mut result = value.to_owned();
    result.replace_range(range, accepted);
    (result, inserted)
}

fn history_action(key: &gpui::Keystroke, mac: bool) -> Option<HistoryAction> {
    let modifiers = key.modifiers;
    let primary = if mac {
        modifiers.platform && !modifiers.control
    } else {
        modifiers.control && !modifiers.platform
    };
    if !primary || modifiers.alt || modifiers.function {
        return None;
    }
    match key.key.as_str() {
        "z" if modifiers.shift => Some(HistoryAction::Redo),
        "z" => Some(HistoryAction::Undo),
        "y" if !mac && !modifiers.shift => Some(HistoryAction::Redo),
        _ => None,
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
        self.finish_composition(cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(
            self.range_for_edit(range),
            &single_line(text),
            Kind::Typing,
            cx,
        );
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
        let text = single_line(text);
        self.history.begin_composition(self.snapshot());
        let inserted = self.replace_raw(range, &text, cx);
        self.marked = (!inserted.is_empty()).then_some(inserted.clone());
        if let Some(selected) = selected {
            let accepted = &self.text[inserted.clone()];
            self.anchor = inserted.start + byte_offset(accepted, selected.start);
            self.caret = inserted.start + byte_offset(accepted, selected.end);
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
        self.input.update(cx, |input, cx| {
            if input.blur.is_none() {
                input.blur = Some(cx.on_blur(&input.focus.clone(), window, |input, _, cx| {
                    input.finish_composition(cx)
                }));
            }
            input.layout = Some((line, bounds, scroll));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_shortcuts_do_not_consume_altgr_or_the_other_platform_modifier() {
        let key = |key: &str, modifiers| gpui::Keystroke {
            key: key.into(),
            key_char: None,
            modifiers,
        };
        let command = gpui::Modifiers {
            platform: true,
            ..Default::default()
        };
        let control = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(
            history_action(&key("z", command), true),
            Some(HistoryAction::Undo)
        );
        assert_eq!(
            history_action(
                &key(
                    "z",
                    gpui::Modifiers {
                        shift: true,
                        ..command
                    }
                ),
                true
            ),
            Some(HistoryAction::Redo)
        );
        assert_eq!(
            history_action(&key("z", control), false),
            Some(HistoryAction::Undo)
        );
        assert_eq!(
            history_action(&key("y", control), false),
            Some(HistoryAction::Redo)
        );
        assert_eq!(
            history_action(
                &key(
                    "z",
                    gpui::Modifiers {
                        shift: true,
                        ..control
                    }
                ),
                false
            ),
            Some(HistoryAction::Redo)
        );
        assert_eq!(
            history_action(
                &key(
                    "z",
                    gpui::Modifiers {
                        alt: true,
                        ..control
                    }
                ),
                false
            ),
            None
        );
        assert_eq!(history_action(&key("z", control), true), None);
        assert_eq!(history_action(&key("z", command), false), None);
        assert_eq!(history_action(&key("y", command), true), None);
    }

    #[test]
    fn limited_composition_ranges_cover_only_the_accepted_unicode_text() {
        let (value, inserted) = replacement("a🐻é", 1..7, "🐻éx", Some(3));
        assert_eq!(value, "a🐻");
        assert_eq!(inserted, 1..5);
        let accepted = &value[inserted.clone()];
        assert_eq!(inserted.start + byte_offset(accepted, 99), value.len());
        let (full, inserted) = replacement("abc", 3..3, "日本", Some(3));
        assert_eq!(full, "abc");
        assert!(inserted.is_empty());
    }

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
