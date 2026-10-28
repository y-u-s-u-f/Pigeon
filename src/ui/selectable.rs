//! Text that takes part in the window's text selection: gpui-base's
//! `TextSelectionLayer`, which the base `Root` mounts, handles dragging,
//! double- and triple-click and shift-click across every participant, and the
//! `Root` copies the selection on ⌘C. Each `Selectable` is one participant.

use std::rc::Rc;

use gpui::{
    A11ySubtreeBuilder, AnyElement, App, Bounds, CursorStyle, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Role,
    SharedString, StyledText, Subscription, TextLayout, Window, accesskit,
    base::{TextSelectionHandle, TextSelectionRegistration, TextSelectionRun},
    fill,
};

/// Orders the reader's participants after any automatically ordered ones
/// (gpui-base numbers those from 1 each frame).
pub const ORDER_BASE: u64 = 1 << 32;

/// Run when a click clears a whole-text (⌘A) selection.
pub type OnClear = Rc<dyn Fn(&mut App)>;

pub struct Selectable {
    id: ElementId,
    text: SharedString,
    layout: TextLayout,
    child: AnyElement,
    order: u64,
    color: Hsla,
    all: Option<OnClear>,
    a11y: A11y,
    /// The text's links, as (range, URL), for VoiceOver (MA-003).
    links: Vec<(std::ops::Range<usize>, SharedString)>,
}

/// What VoiceOver reads the text as (MA-001).
#[derive(Clone, Copy)]
enum A11y {
    Text,
    Heading(usize),
    /// A parent already says it.
    Silent,
}

impl Selectable {
    /// `text` is what `styled` lays out; `wrap` may add interaction around it.
    pub fn new(
        id: impl Into<ElementId>,
        text: SharedString,
        styled: StyledText,
        wrap: impl FnOnce(StyledText) -> AnyElement,
    ) -> Self {
        Self {
            id: id.into(),
            text,
            layout: styled.layout().clone(),
            child: wrap(styled),
            order: ORDER_BASE,
            color: gpui::transparent_black(),
            all: None,
            a11y: A11y::Text,
            links: Vec::new(),
        }
    }

    pub fn plain(id: impl Into<ElementId>, text: impl Into<SharedString>, color: Hsla) -> Self {
        let text = text.into();
        Self::new(id, text.clone(), StyledText::new(text), |t| {
            t.into_any_element()
        })
        .color(color)
    }

    /// Reading order among the window's selectable text.
    pub fn order(mut self, order: u64) -> Self {
        self.order = order;
        self
    }

    /// The selection's background.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = color;
        self
    }

    /// Read as a heading of `level` (1 to 6).
    pub fn heading(mut self, level: usize) -> Self {
        self.a11y = A11y::Heading(level);
        self
    }

    /// Left out of the accessibility tree, where a parent reads the same.
    pub fn silent(mut self) -> Self {
        self.a11y = A11y::Silent;
        self
    }

    /// The text's links, each read as a link inside it.
    pub fn links(mut self, links: Vec<(std::ops::Range<usize>, SharedString)>) -> Self {
        self.links = links;
        self
    }

    /// Selects the whole text until a click clears it, which calls `on_clear`.
    pub fn select_all(mut self, on_clear: Option<OnClear>) -> Self {
        self.all = on_clear;
        self
    }

    fn paint_range(&self, start: usize, end: usize, window: &mut Window) {
        let (Some(start), Some(end)) = (
            self.layout.position_for_index(start),
            self.layout.position_for_index(end),
        ) else {
            return;
        };
        let bounds = self.layout.bounds();
        let line = self.layout.line_height();
        let quads = if start.y == end.y {
            vec![Bounds::from_corners(start, Point::new(end.x, end.y + line))]
        } else {
            let mut quads = vec![Bounds::from_corners(
                start,
                Point::new(bounds.right(), start.y + line),
            )];
            if end.y > start.y + line {
                quads.push(Bounds::from_corners(
                    Point::new(bounds.left(), start.y + line),
                    Point::new(bounds.right(), end.y),
                ));
            }
            quads.push(Bounds::from_corners(
                Point::new(bounds.left(), end.y),
                Point::new(end.x, end.y + line),
            ));
            quads
        };
        for quad in quads {
            window.paint_quad(fill(quad, self.color));
        }
    }
}

impl IntoElement for Selectable {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Selectable {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<Role> {
        match self.a11y {
            // Linked text is an `InteractiveText`, which reads the text
            // itself; this holds its links.
            A11y::Text if !self.links.is_empty() => Some(Role::Paragraph),
            A11y::Text => Some(Role::Label),
            A11y::Heading(_) => Some(Role::Heading),
            A11y::Silent => None,
        }
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        match self.a11y {
            A11y::Heading(level) => {
                node.set_label(self.text.to_string());
                node.set_level(level);
            }
            A11y::Text if self.links.is_empty() => node.set_value(self.text.to_string()),
            _ => {}
        }
    }

    fn a11y_synthetic_children(&mut self, _: &mut Hitbox, builder: &mut A11ySubtreeBuilder) {
        for (ix, (range, url)) in self.links.iter().enumerate() {
            let mut link = accesskit::Node::new(Role::Link);
            if let Some(text) = self.text.get(range.clone()) {
                link.set_label(text.to_string());
            }
            link.set_url(url.to_string());
            let id = builder.synthetic_node_id(ix);
            builder.push_child(id, link);
        }
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let handle = window.with_element_state(
            id.expect("Selectable has an id"),
            |retained: Option<(TextSelectionHandle, Subscription)>, window| {
                let state = retained.unwrap_or_else(|| {
                    let handle = TextSelectionHandle::new(self.text.clone(), cx);
                    // Repaint as a drag moves the selection.
                    let repaint = handle.refresh_window_on_change(window, cx);
                    (handle, repaint)
                });
                (state.0.clone(), state)
            },
        );
        (self.child.request_layout(window, cx), handle)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_document_order(self.order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.set_cursor_style(CursorStyle::IBeam, hitbox);
        let run = TextSelectionRun::new(self.text.clone(), self.layout.clone(), bounds)
            .with_document_order(self.order);
        let projection = handle.update_runs(&[run], cx);
        match self.all.clone() {
            Some(on_clear) => {
                // Copy falls back to the whole text while it is selected locally.
                handle.set_fallback_copy_text(self.text.to_string(), cx);
                if !handle.has_local_selection(cx) {
                    handle.set_local_selection(true, cx);
                    handle.clear_with(move |cx| on_clear(cx), cx);
                }
                self.paint_range(0, self.text.len(), window);
            }
            None => {
                for range in projection.ranges().iter().flatten() {
                    self.paint_range(range.start, range.end, window);
                }
            }
        }
        self.child.paint(window, cx);
    }
}
