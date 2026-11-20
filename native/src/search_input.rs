use crate::text_history::{History, Kind, Snapshot};
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, GlobalElementId, InspectorElementId, LayoutId,
    MouseButton, Pixels, Point, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window,
    div, fill, point, prelude::*, px, relative, rgba, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

gpui::actions!(
    astral_text,
    [
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        PasteMatchStyle,
        DeleteSelection,
        SelectAll
    ]
);

pub fn edit_menu() -> gpui::Menu {
    use gpui::{Menu, MenuItem, OsAction};
    let mut items = vec![
        MenuItem::os_action("Undo", Undo, OsAction::Undo),
        MenuItem::os_action("Redo", Redo, OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action("Cut", Cut, OsAction::Cut),
        MenuItem::os_action("Copy", Copy, OsAction::Copy),
        MenuItem::os_action("Paste", Paste, OsAction::Paste),
    ];
    if cfg!(target_os = "macos") {
        items.push(MenuItem::os_action(
            "Paste and Match Style",
            PasteMatchStyle,
            OsAction::Paste,
        ));
    }
    items.push(MenuItem::action("Delete", DeleteSelection));
    if !cfg!(target_os = "macos") {
        items.push(MenuItem::separator());
    }
    items.push(MenuItem::os_action(
        "Select All",
        SelectAll,
        OsAction::SelectAll,
    ));
    Menu {
        name: "Edit".into(),
        items,
    }
}

pub fn bind_menu_keys(cx: &mut App) {
    cx.bind_keys(menu_keys(cfg!(target_os = "macos")));
}

fn menu_keys(mac: bool) -> Vec<gpui::KeyBinding> {
    use gpui::KeyBinding;
    vec![
        KeyBinding::new(
            if mac { "cmd-z" } else { "ctrl-z" },
            Undo,
            Some("AstralText"),
        ),
        KeyBinding::new(
            if mac { "cmd-shift-z" } else { "ctrl-y" },
            Redo,
            Some("AstralText"),
        ),
        KeyBinding::new(
            if mac { "cmd-x" } else { "ctrl-x" },
            Cut,
            Some("AstralText"),
        ),
        KeyBinding::new(
            if mac { "cmd-c" } else { "ctrl-c" },
            Copy,
            Some("AstralText"),
        ),
        KeyBinding::new(
            if mac { "cmd-v" } else { "ctrl-v" },
            Paste,
            Some("AstralText"),
        ),
        KeyBinding::new(
            if mac { "cmd-a" } else { "ctrl-a" },
            SelectAll,
            Some("AstralText"),
        ),
        KeyBinding::new(
            if mac {
                "cmd-alt-shift-v"
            } else {
                "ctrl-shift-v"
            },
            PasteMatchStyle,
            Some("AstralText"),
        ),
    ]
}

pub struct SearchInput {
    pub focus: FocusHandle,
    text: SharedString,
    anchor: usize,
    caret: usize,
    marked: Option<Range<usize>>,
    layout: Option<(ShapedLine, Bounds<Pixels>, Pixels)>,
    dragging: Option<SelectionGesture>,
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
            dragging: None,
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
        self.dragging = None;
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
        self.dragging = None;
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
        self.dragging = None;
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
        self.dragging = None;
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
                self.history_command(action, cx);
                cx.stop_propagation();
            }
            return;
        }
        // Native composition owns its candidate selection and deletion keys.
        if self.history.composing() {
            return;
        }
        let mac = cfg!(target_os = "macos");
        let Some(action) = edit_action(key, mac) else {
            return;
        };
        self.edit(action, modifiers.shift, cx);
        cx.stop_propagation();
    }

    fn history_command(&mut self, action: HistoryAction, cx: &mut Context<Self>) {
        let snapshot = match action {
            HistoryAction::Undo => self.history.undo(),
            HistoryAction::Redo => self.history.redo(),
        };
        if let Some(snapshot) = snapshot {
            self.restore(snapshot, cx);
        }
    }

    fn edit(&mut self, action: EditAction, extend: bool, cx: &mut Context<Self>) {
        if self.history.composing() {
            return;
        }
        let selection = self.selected();
        match action {
            EditAction::SelectAll => {
                self.anchor = 0;
                self.select(self.text.len(), true, cx);
            }
            EditAction::Copy | EditAction::Cut => {
                if !selection.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.text[selection.clone()].into(),
                    ));
                }
                if action == EditAction::Cut && !selection.is_empty() {
                    self.replace(selection, "", Kind::Separate, cx);
                }
            }
            EditAction::Paste => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(selection, &single_line(&text), Kind::Separate, cx);
                }
            }
            EditAction::Move(motion) => {
                let offset = match motion {
                    Motion::Previous if !extend && !selection.is_empty() => selection.start,
                    Motion::Next if !extend && !selection.is_empty() => selection.end,
                    _ => motion.offset(&self.text, self.caret, cfg!(target_os = "macos")),
                };
                self.select(offset, extend, cx);
            }
            EditAction::Delete(motion) => {
                let range = if !selection.is_empty() {
                    selection
                } else {
                    let other = motion.offset(&self.text, self.caret, cfg!(target_os = "macos"));
                    self.caret.min(other)..self.caret.max(other)
                };
                self.replace(
                    range,
                    "",
                    if matches!(
                        motion,
                        Motion::Previous | Motion::PreviousWord | Motion::Start
                    ) {
                        Kind::Backspace
                    } else {
                        Kind::ForwardDelete
                    },
                    cx,
                );
            }
        }
    }
}

#[derive(Clone)]
struct SelectionGesture {
    original: Range<usize>,
    granularity: usize,
}

impl SelectionGesture {
    fn drag(&self, text: &str, offset: usize) -> (usize, usize) {
        let current = click_selection(text, offset, self.granularity);
        if offset < self.original.start {
            (self.original.end, current.start)
        } else if offset >= self.original.end {
            (self.original.start, current.end)
        } else {
            (self.original.start, self.original.end)
        }
    }
}

fn click_selection(text: &str, offset: usize, count: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    match count {
        0 | 1 => offset..offset,
        2 => {
            // At the trailing edge of a field, select its final word rather
            // than leaving a double click with an empty caret selection.
            let hit = if offset == text.len() {
                previous_grapheme(text, offset)
            } else {
                offset
            };
            text.split_word_bound_indices()
                .find(|(index, part)| *index <= hit && hit < index + part.len())
                .map_or(offset..offset, |(index, part)| index..index + part.len())
        }
        _ => 0..text.len(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Motion {
    Previous,
    Next,
    PreviousWord,
    NextWord,
    Start,
    End,
}

impl Motion {
    fn offset(self, text: &str, caret: usize, mac: bool) -> usize {
        match self {
            Self::Previous => previous_grapheme(text, caret),
            Self::Next => next_grapheme(text, caret),
            Self::Start => 0,
            Self::End => text.len(),
            Self::PreviousWord => text
                .split_word_bound_indices()
                .rev()
                .find(|(index, part)| *index < caret && !part.chars().all(char::is_whitespace))
                .map_or(0, |(index, _)| index),
            Self::NextWord => {
                let mut parts = text
                    .split_word_bound_indices()
                    .filter(|(index, part)| index + part.len() > caret)
                    .peekable();
                let mut end = caret;
                // Mac stops at the next word end; Windows skips trailing spaces.
                while let Some((index, part)) = parts.next() {
                    end = index + part.len();
                    if !mac || !part.chars().all(char::is_whitespace) {
                        if !mac {
                            while let Some((index, part)) = parts.peek() {
                                if !part.chars().all(char::is_whitespace) {
                                    break;
                                }
                                end = index + part.len();
                                parts.next();
                            }
                        }
                        break;
                    }
                }
                end
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum EditAction {
    SelectAll,
    Copy,
    Cut,
    Paste,
    Move(Motion),
    Delete(Motion),
}

fn edit_action(key: &gpui::Keystroke, mac: bool) -> Option<EditAction> {
    use EditAction::*;
    use Motion::*;
    let m = key.modifiers;
    // AppKit's StandardKeyBinding.dict supplies these editing commands in
    // Electron. GPUI redispatches the keystroke instead of the Cocoa selector.
    if mac && m.control && !m.platform && !m.alt && !m.function {
        return match key.key.to_ascii_lowercase().as_str() {
            "a" => Some(Move(Start)),
            "e" => Some(Move(End)),
            "b" => Some(Move(Previous)),
            "f" => Some(Move(Next)),
            "h" if !m.shift => Some(Delete(Previous)),
            "d" if !m.shift => Some(Delete(Next)),
            _ => None,
        };
    }
    if m.function {
        // Fn is reported for actual function keys on some keyboards; arrows,
        // Home/End and Delete remain valid editing keys with that flag.
        if !matches!(
            key.key.as_str(),
            "left" | "right" | "home" | "end" | "backspace" | "delete"
        ) {
            return None;
        }
    }
    let primary = if mac {
        m.platform && !m.control && !m.alt
    } else {
        m.control && !m.platform && !m.alt
    };
    let plain = !m.platform && !m.control && !m.alt;
    let word = if mac {
        m.alt && !m.platform && !m.control
    } else {
        m.control && !m.alt && !m.platform
    };
    let line = if mac { primary } else { plain || primary };
    // Match browser shortcuts exactly enough to leave AltGr character input
    // alone. Ctrl-Alt-A/C/V must never select, copy or paste on Windows.
    match key.key.as_str() {
        "a" if primary && !m.shift => Some(SelectAll),
        "c" if primary && !m.shift => Some(Copy),
        "x" if primary && !m.shift => Some(Cut),
        "v" if primary => Some(Paste),
        "insert" if !mac && primary && !m.shift => Some(Copy),
        "insert" if !mac && plain && m.shift => Some(Paste),
        "delete" if !mac && plain && m.shift => Some(Cut),
        "left" if word => Some(Move(PreviousWord)),
        "right" if word => Some(Move(NextWord)),
        "left" if mac && primary => Some(Move(Start)),
        "right" if mac && primary => Some(Move(End)),
        "left" if plain => Some(Move(Previous)),
        "right" if plain => Some(Move(Next)),
        "home" if line || plain => Some(Move(Start)),
        "end" if line || plain => Some(Move(End)),
        "backspace" if word && !m.shift => Some(Delete(PreviousWord)),
        "delete" if word && !m.shift => Some(Delete(NextWord)),
        "backspace" if mac && primary && !m.shift => Some(Delete(Start)),
        "backspace" if plain => Some(Delete(Previous)),
        "delete" if plain && !m.shift => Some(Delete(Next)),
        _ => None,
    }
}

fn previous_grapheme(text: &str, caret: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index < caret)
        .last()
        .unwrap_or(0)
}

fn next_grapheme(text: &str, caret: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index > caret)
        .unwrap_or(text.len())
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
    if !mac
        && key.key == "backspace"
        && modifiers.alt
        && !modifiers.control
        && !modifiers.platform
        && !modifiers.function
    {
        return Some(if modifiers.shift {
            HistoryAction::Redo
        } else {
            HistoryAction::Undo
        });
    }
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
            .key_context("AstralText")
            .flex_1()
            .min_w_0()
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(SearchInput::edit_key))
            .when(input.history.can_undo(), |field| {
                field.on_action(cx.listener(|input, _: &Undo, _, cx| {
                    input.history_command(HistoryAction::Undo, cx)
                }))
            })
            .when(input.history.can_redo(), |field| {
                field.on_action(cx.listener(|input, _: &Redo, _, cx| {
                    input.history_command(HistoryAction::Redo, cx)
                }))
            })
            .when(!input.history.composing(), |field| {
                field
                    .on_action(cx.listener(|input, _: &Paste, _, cx| {
                        input.edit(EditAction::Paste, false, cx)
                    }))
                    .on_action(cx.listener(|input, _: &PasteMatchStyle, _, cx| {
                        input.edit(EditAction::Paste, false, cx)
                    }))
                    .on_action(cx.listener(|input, _: &SelectAll, _, cx| {
                        input.edit(EditAction::SelectAll, false, cx)
                    }))
                    .when(!input.selected().is_empty(), |field| {
                        field
                            .on_action(cx.listener(|input, _: &Copy, _, cx| {
                                input.edit(EditAction::Copy, false, cx)
                            }))
                            .on_action(cx.listener(|input, _: &Cut, _, cx| {
                                input.edit(EditAction::Cut, false, cx)
                            }))
                            .on_action(cx.listener(|input, _: &DeleteSelection, _, cx| {
                                // Menu Delete removes the selected range; it
                                // neither cuts to the clipboard nor backspaces.
                                input.replace(input.selected(), "", Kind::Separate, cx);
                            }))
                    })
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|input, event: &gpui::MouseDownEvent, window, cx| {
                    input.focus.focus(window);
                    let offset = input.hit(event.position);
                    let count = event.click_count;
                    let original = click_selection(&input.text, offset, count);
                    if count > 1 {
                        input.anchor = original.start;
                        input.select(original.end, true, cx);
                    } else {
                        input.select(offset, event.modifiers.shift, cx);
                    }
                    input.dragging = Some(SelectionGesture {
                        original: if count > 1 {
                            original
                        } else {
                            input.anchor..input.anchor
                        },
                        granularity: count,
                    });
                }),
            )
            .on_mouse_move(cx.listener(|input, event: &gpui::MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    input.dragging = None;
                    return;
                }
                if let Some(gesture) = &input.dragging {
                    let (anchor, caret) = gesture.drag(&input.text, input.hit(event.position));
                    input.anchor = anchor;
                    input.select(caret, true, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|input, _, _, _| input.dragging = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|input, _, _, _| input.dragging = None),
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
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(text, font_size, &[run], None);
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
    fn multiple_click_selection_preserves_words_combining_text_and_the_single_line() {
        let text = "Raid  Joa\u{303}o 🐻";
        assert_eq!(click_selection(text, 2, 1), 2..2);
        assert_eq!(&text[click_selection(text, 2, 2)], "Raid");
        assert_eq!(&text[click_selection(text, 4, 2)], "  ");
        assert_eq!(&text[click_selection(text, 8, 2)], "Joa\u{303}o");
        assert_eq!(&text[click_selection(text, text.len(), 2)], "🐻");
        assert_eq!(click_selection(text, 7, 3), 0..text.len());
        assert_eq!(click_selection("", 99, 2), 0..0);
    }

    #[test]
    fn word_drag_extends_by_whole_words_and_retains_the_original_selection_when_reversing() {
        let text = "Raid  healing setup";
        let gesture = SelectionGesture {
            original: 6..13,
            granularity: 2,
        };
        assert_eq!(gesture.drag(text, 9), (6, 13));
        assert_eq!(gesture.drag(text, 16), (6, text.len()));
        assert_eq!(gesture.drag(text, 2), (13, 0));
        assert_eq!(gesture.drag(text, 9), (6, 13));
        let character = SelectionGesture {
            original: 6..6,
            granularity: 1,
        };
        assert_eq!(character.drag(text, 2), (6, 2));
        assert_eq!(character.drag(text, 16), (6, 16));
        let line = SelectionGesture {
            original: 0..text.len(),
            granularity: 3,
        };
        assert_eq!(line.drag(text, 2), (0, text.len()));
    }

    #[test]
    fn edit_menu_bindings_are_confined_to_text_focus_and_use_platform_accelerators() {
        for mac in [true, false] {
            let keymap = gpui::Keymap::new(menu_keys(mac));
            let text = gpui::KeyContext::parse("AstralText").unwrap();
            let workspace = gpui::KeyContext::parse("Workspace").unwrap();
            let undo = gpui::Keystroke::parse(if mac { "cmd-z" } else { "ctrl-z" }).unwrap();
            let (matches, pending) = keymap.bindings_for_input(&[undo.clone()], &[text.clone()]);
            assert!(!pending);
            assert_eq!(matches.len(), 1);
            assert!(matches[0].action().as_any().is::<Undo>());
            assert!(
                keymap
                    .bindings_for_input(&[undo], &[workspace])
                    .0
                    .is_empty()
            );
            let redo = gpui::Keystroke::parse(if mac { "cmd-shift-z" } else { "ctrl-y" }).unwrap();
            let (matches, _) = keymap.bindings_for_input(&[redo], &[text.clone()]);
            assert_eq!(matches.len(), 1);
            assert!(matches[0].action().as_any().is::<Redo>());
            let select = gpui::Keystroke::parse(if mac { "cmd-a" } else { "ctrl-a" }).unwrap();
            let (matches, _) = keymap.bindings_for_input(&[select], &[text.clone()]);
            assert!(matches[0].action().as_any().is::<SelectAll>());
            let paste = gpui::Keystroke::parse(if mac {
                "cmd-alt-shift-v"
            } else {
                "ctrl-shift-v"
            })
            .unwrap();
            let (matches, _) = keymap.bindings_for_input(&[paste], &[text.clone()]);
            assert_eq!(matches.len(), 1);
            assert!(matches[0].action().as_any().is::<PasteMatchStyle>());
            assert!(
                keymap
                    .bindings_for_input(&[gpui::Keystroke::parse("ctrl-alt-a").unwrap()], &[text])
                    .0
                    .is_empty()
            );
        }
    }

    fn key(name: &str, modifiers: gpui::Modifiers) -> gpui::Keystroke {
        gpui::Keystroke {
            key: name.into(),
            key_char: None,
            modifiers,
        }
    }

    #[test]
    fn character_navigation_keeps_accents_flags_and_joined_emoji_together() {
        let pieces = ["J", "o", "a\u{303}", "o", "🇵🇹", "👩🏽‍💻", "x"];
        let text = pieces.concat();
        let mut offset = 0;
        for piece in pieces {
            let next = next_grapheme(&text, offset);
            assert_eq!(&text[offset..next], piece);
            assert_eq!(previous_grapheme(&text, next), offset);
            offset = next;
        }
        assert_eq!(previous_grapheme(&text, 0), 0);
        assert_eq!(next_grapheme(&text, text.len()), text.len());
        // Native selection offsets can land inside a multi-scalar cluster.
        assert_eq!(previous_grapheme("👩🏽‍💻x", "👩".len()), 0);
        assert_eq!(next_grapheme("👩🏽‍💻x", "👩".len()), "👩🏽‍💻".len());
    }

    #[test]
    fn word_navigation_retains_full_word_context_and_platform_space_behavior() {
        use Motion::*;
        let text = "Raid  can't  João";
        assert_eq!(NextWord.offset(text, 0, true), 4);
        assert_eq!(NextWord.offset(text, 0, false), 6);
        assert_eq!(NextWord.offset(text, 4, true), 11);
        assert_eq!(NextWord.offset(text, 4, false), 6);
        assert_eq!(NextWord.offset(text, 8, true), 11);
        assert_eq!(PreviousWord.offset(text, 10, true), 6);
        assert_eq!(PreviousWord.offset(text, 13, false), 6);
        assert_eq!(PreviousWord.offset(text, text.len(), false), 13);
        assert_eq!(NextWord.offset(text, text.len(), false), text.len());
        assert_eq!(PreviousWord.offset(text, 0, true), 0);
    }

    #[test]
    fn editing_shortcuts_use_platform_modifiers_and_leave_altgr_alone() {
        use EditAction::*;
        use Motion::*;
        let plain = gpui::Modifiers::default();
        let command = gpui::Modifiers {
            platform: true,
            ..plain
        };
        let control = gpui::Modifiers {
            control: true,
            ..plain
        };
        let option = gpui::Modifiers { alt: true, ..plain };
        for name in ["a", "c", "x", "v", "left", "right", "backspace", "delete"] {
            assert_eq!(
                edit_action(
                    &key(
                        name,
                        gpui::Modifiers {
                            alt: true,
                            ..control
                        }
                    ),
                    false
                ),
                None
            );
        }
        assert_eq!(
            edit_action(&key("left", option), true),
            Some(Move(PreviousWord))
        );
        assert_eq!(
            edit_action(&key("left", control), false),
            Some(Move(PreviousWord))
        );
        assert_eq!(edit_action(&key("left", command), true), Some(Move(Start)));
        assert_eq!(edit_action(&key("left", command), false), None);
        assert_eq!(
            edit_action(&key("backspace", command), true),
            Some(Delete(Start))
        );
        assert_eq!(
            edit_action(&key("delete", control), false),
            Some(Delete(NextWord))
        );
        assert_eq!(edit_action(&key("c", control), true), None);
        assert_eq!(edit_action(&key("c", command), true), Some(Copy));
        assert_eq!(
            edit_action(
                &key(
                    "v",
                    gpui::Modifiers {
                        shift: true,
                        ..control
                    }
                ),
                false
            ),
            Some(Paste)
        );
        assert_eq!(
            edit_action(
                &key(
                    "delete",
                    gpui::Modifiers {
                        shift: true,
                        ..plain
                    }
                ),
                false
            ),
            Some(Cut)
        );
        assert_eq!(
            edit_action(
                &key(
                    "insert",
                    gpui::Modifiers {
                        shift: true,
                        ..plain
                    }
                ),
                false
            ),
            Some(Paste)
        );
        assert_eq!(edit_action(&key("insert", control), false), Some(Copy));
    }

    #[test]
    fn mac_control_letter_commands_do_not_replace_windows_select_all_or_plain_typing() {
        use EditAction::*;
        use Motion::*;
        let control = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        for (name, action) in [
            ("a", Move(Start)),
            ("e", Move(End)),
            ("b", Move(Previous)),
            ("f", Move(Next)),
            ("h", Delete(Previous)),
            ("d", Delete(Next)),
        ] {
            assert_eq!(edit_action(&key(name, control), true), Some(action));
            assert_eq!(edit_action(&key(name, Default::default()), true), None);
        }
        assert_eq!(
            edit_action(
                &key(
                    "A",
                    gpui::Modifiers {
                        shift: true,
                        ..control
                    }
                ),
                true
            ),
            Some(Move(Start))
        );
        assert_eq!(edit_action(&key("a", control), false), Some(SelectAll));
        assert_eq!(
            edit_action(
                &key(
                    "d",
                    gpui::Modifiers {
                        alt: true,
                        ..control
                    }
                ),
                true
            ),
            None
        );
        assert_eq!(
            edit_action(
                &key(
                    "h",
                    gpui::Modifiers {
                        shift: true,
                        ..control
                    }
                ),
                true
            ),
            None
        );
    }

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
        let alt = gpui::Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            history_action(&key("backspace", alt), false),
            Some(HistoryAction::Undo)
        );
        assert_eq!(
            history_action(
                &key("backspace", gpui::Modifiers { shift: true, ..alt }),
                false
            ),
            Some(HistoryAction::Redo)
        );
        assert_eq!(history_action(&key("backspace", alt), true), None);
        assert_eq!(
            history_action(
                &key(
                    "backspace",
                    gpui::Modifiers {
                        control: true,
                        ..alt
                    }
                ),
                false
            ),
            None
        );
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
