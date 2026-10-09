use std::cell::Cell;
use std::rc::Rc;

use super::tab_manager::TabManager;
use super::types::{DocumentId, DocumentMetaSnapshot, DocumentState};
use dbflux_components::composites::{MenuItem, document_tab, document_tab_bar, document_tab_title};
use dbflux_components::controls::Button;
use dbflux_components::icons::AppIcon;
use dbflux_components::primitives::Text;
use dbflux_components::primitives::{Icon, Status, StatusIndicator};
use dbflux_components::tokens::{ChromeColors, FontSizes, Radii, Spacing, TabMetrics};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::tooltip::Tooltip;
use uuid::Uuid;

/// Height of the band above each tab that names the database the tab
/// belongs to. With the 30 px chip below it the column fills the 46 px bar,
/// so the bar keeps its height and everything anchored under it stays put.
const TAB_GROUP_BAND: Pixels = Spacing::LG;

/// Narrowest a tab may get. Tabs never shrink past this, however many are
/// open — the strip scrolls instead, because a row of four-letter stumps
/// tells the user nothing about which table each tab holds.
const TAB_MIN_WIDTH: Pixels = px(140.0);

/// Widest an inactive tab gets before its title is ellipsized.
const TAB_MAX_WIDTH: Pixels = px(220.0);

/// Widest the active tab gets. Larger than the rest so the table you are
/// actually looking at shows its whole name.
const TAB_ACTIVE_MAX_WIDTH: Pixels = px(360.0);

/// Width the tab context menu is assumed to take when deciding whether it
/// fits: the floor plus room for the longest label ("Close Tabs to the Right").
pub const TAB_MENU_WIDTH: Pixels = px(220.0);

/// Space kept between the tab context menu and the window edge.
const TAB_MENU_EDGE_GAP: Pixels = Spacing::SM;

/// Left edge for the tab context menu opened at `click_x`.
///
/// Anchored at the click, pulled left when the menu would otherwise run past
/// the right edge of the window — right-clicking the last tab used to open a
/// menu half outside the window, where the items were unreachable.
pub fn clamp_tab_menu_left(click_x: Pixels, menu_width: Pixels, viewport_width: Pixels) -> Pixels {
    let rightmost = viewport_width - menu_width - TAB_MENU_EDGE_GAP;
    // `max` last: in a window narrower than the menu, staying attached to the
    // left edge beats sliding off the left one.
    click_x.min(rightmost).max(TAB_MENU_EDGE_GAP)
}

/// Title for the application window: the active document, the database it
/// belongs to, then the product name — the order DBeaver and DbGate use, so
/// the part that changes is the part the window list shows first.
pub fn window_title(document: Option<(&str, Option<&str>)>, product: &str) -> String {
    match document {
        Some((title, Some(group))) => format!("{title} - {group} - {product}"),
        Some((title, None)) => format!("{title} - {product}"),
        None => product.to_string(),
    }
}

/// A tab being dragged to a new position in the bar.
#[derive(Clone)]
pub struct TabDrag {
    /// Where the tab sits right now — the source index for the move.
    index: usize,
    label: SharedString,
}

/// The label that follows the cursor while a tab is dragged.
struct TabDragPreview {
    label: SharedString,
}

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .bg(theme.tab_bar)
            .border_1()
            .border_color(theme.drag_border)
            .rounded(Radii::SM)
            .px(Spacing::SM)
            .py(Spacing::XS)
            .shadow_md()
            .child(Text::body(self.label.clone()).font_size(FontSizes::SM))
    }
}

/// What makes two neighbouring tabs share a band: same connection, same
/// database.
#[derive(Clone, PartialEq, Eq)]
struct TabGroupKey {
    connection_id: Option<Uuid>,
    database: SharedString,
}

impl TabGroupKey {
    /// The band's colour, derived from the names.
    ///
    /// Drawn from the theme's chart palette so it fits either theme, and from
    /// a hash rather than a running counter so a database keeps its colour as
    /// tabs open and close around it.
    fn color(&self, theme: &gpui_component::theme::Theme) -> Hsla {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.connection_id.hash(&mut hasher);
        self.database.hash(&mut hasher);
        match hasher.finish() % 5 {
            0 => theme.chart_1,
            1 => theme.chart_2,
            2 => theme.chart_3,
            3 => theme.chart_4,
            _ => theme.chart_5,
        }
    }
}

/// The band rendered above one tab: coloured when the tab has a database,
/// labelled only on the first tab of a run so the name reads once per group.
struct TabGroupBand {
    color: Option<Hsla>,
    label: Option<SharedString>,
}

impl TabGroupBand {
    fn new(
        group: Option<&TabGroupKey>,
        starts_group: bool,
        theme: &gpui_component::theme::Theme,
    ) -> Self {
        Self {
            color: group.map(|group| group.color(theme)),
            label: group
                .filter(|_| starts_group)
                .map(|group| group.database.clone()),
        }
    }
}

#[allow(dead_code)]
pub struct TabBar {
    tab_manager: Entity<TabManager>,
    focus_handle: FocusHandle,

    context_menu: Option<TabContextMenu>,

    /// Center X of the active tab, updated each render via canvas measurement.
    active_tab_center_x: Rc<Cell<Pixels>>,

    // Drag state (for future drag & drop support)
    dragging_tab: Option<DocumentId>,
    drop_target_index: Option<usize>,

    /// Horizontal scroll of the tab strip, so the active tab can be brought
    /// into view when there are more tabs than fit.
    scroll_handle: ScrollHandle,
    /// The tab that was active at the last render; a change means the new
    /// one has to be scrolled into view.
    last_active_id: Option<DocumentId>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct TabContextMenu {
    pub tab_id: DocumentId,
    pub tab_index: usize,
    /// X position from the mouse click (window-absolute).
    pub position_x: Pixels,
    pub selected_index: usize,
}

pub const TAB_MENU_CLOSE: usize = 0;
pub const TAB_MENU_CLOSE_OTHERS: usize = 1;
pub const TAB_MENU_CLOSE_ALL: usize = 2;
#[allow(dead_code)]
pub const TAB_MENU_SEPARATOR: usize = 3;
pub const TAB_MENU_CLOSE_LEFT: usize = 4;
pub const TAB_MENU_CLOSE_RIGHT: usize = 5;

impl TabBar {
    pub fn new(tab_manager: Entity<TabManager>, cx: &mut Context<Self>) -> Self {
        Self {
            tab_manager,
            focus_handle: cx.focus_handle(),
            context_menu: None,
            active_tab_center_x: Rc::new(Cell::new(px(0.0))),
            dragging_tab: None,
            drop_target_index: None,
            scroll_handle: ScrollHandle::new(),
            last_active_id: None,
        }
    }

    pub fn context_menu_state(&self) -> Option<&TabContextMenu> {
        self.context_menu.as_ref()
    }

    pub fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        cx.notify();
    }

    pub fn context_menu_hover_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(ref mut menu) = self.context_menu
            && menu.selected_index != index
        {
            menu.selected_index = index;
            cx.notify();
        }
    }

    pub fn context_menu_execute_at(&mut self, action_index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };

        let tab_id = menu.tab_id;

        match action_index {
            TAB_MENU_CLOSE => cx.emit(TabBarEvent::CloseTab(tab_id)),
            TAB_MENU_CLOSE_OTHERS => cx.emit(TabBarEvent::CloseOtherTabs(tab_id)),
            TAB_MENU_CLOSE_ALL => cx.emit(TabBarEvent::CloseAllTabs),
            TAB_MENU_CLOSE_LEFT => cx.emit(TabBarEvent::CloseTabsToLeft(tab_id)),
            TAB_MENU_CLOSE_RIGHT => cx.emit(TabBarEvent::CloseTabsToRight(tab_id)),
            _ => {}
        }

        cx.notify();
    }

    pub fn build_tab_menu_items() -> Vec<MenuItem> {
        vec![
            MenuItem::new(dbflux_i18n::t!("document.tabs.menu.close")).icon(AppIcon::X),
            MenuItem::new(dbflux_i18n::t!("document.tabs.menu.close_others")).icon(AppIcon::X),
            MenuItem::new(dbflux_i18n::t!("document.tabs.menu.close_all")).icon(AppIcon::X),
            MenuItem::separator(),
            MenuItem::new(dbflux_i18n::t!("document.tabs.menu.close_left"))
                .icon(AppIcon::ChevronLeft),
            MenuItem::new(dbflux_i18n::t!("document.tabs.menu.close_right"))
                .icon(AppIcon::ChevronRight),
        ]
    }

    pub fn has_context_menu_open(&self) -> bool {
        self.context_menu.is_some()
    }

    /// Requests a close for one tab through the tab bar's single close route.
    ///
    /// The close button, a middle-click, and the context menu's Close item all
    /// emit `TabBarEvent::CloseTab`, so the workspace applies one close policy
    /// to every gesture instead of removing the tab from the bar directly.
    pub fn request_close(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        cx.emit(TabBarEvent::CloseTab(id));
    }

    /// Commits the input the active document still holds in an open editor,
    /// before a right-click on a tab opens the tab menu.
    ///
    /// The click moves focus out of the document, and an inline editor that
    /// loses focus drops what was typed into it before any menu item can run.
    /// Every item of this menu closes tabs, the active one included, so the
    /// value is committed first and the close it leads to asks about it. Input
    /// the document cannot commit stays where it is, and the close refuses it.
    fn commit_active_pending_input(&mut self, cx: &mut Context<Self>) {
        self.tab_manager.update(cx, |manager, cx| {
            if let Some(tab) = manager.active_id().and_then(|id| manager.document(id)) {
                tab.as_pane().commit_pending_input(cx);
            }
        });
    }

    pub fn open_context_menu_for_active(&mut self, cx: &mut Context<Self>) {
        let manager = self.tab_manager.read(cx);
        let Some(active_id) = manager.active_id() else {
            return;
        };

        let active_index = manager
            .documents()
            .iter()
            .position(|d| d.id() == active_id)
            .unwrap_or(0);

        self.context_menu = Some(TabContextMenu {
            tab_id: active_id,
            tab_index: active_index,
            position_x: self.active_tab_center_x.get(),
            selected_index: 0,
        });
        cx.notify();
    }

    pub fn context_menu_select_next(&mut self, cx: &mut Context<Self>) {
        let Some(ref mut menu) = self.context_menu else {
            return;
        };

        let items = Self::build_tab_menu_items();
        menu.selected_index = next_actionable_index(menu.selected_index, &items);
        cx.notify();
    }

    pub fn context_menu_select_prev(&mut self, cx: &mut Context<Self>) {
        let Some(ref mut menu) = self.context_menu else {
            return;
        };

        let items = Self::build_tab_menu_items();
        menu.selected_index = prev_actionable_index(menu.selected_index, &items);
        cx.notify();
    }

    pub fn context_menu_execute(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };

        self.context_menu_execute_at(menu.selected_index, cx);
    }
}

/// Returns the next non-separator index after `current`, or `current` if at the end.
pub fn next_actionable_index(current: usize, items: &[MenuItem]) -> usize {
    let mut idx = current + 1;
    while idx < items.len() {
        if !items[idx].is_separator {
            return idx;
        }
        idx += 1;
    }
    current
}

/// Returns the previous non-separator index before `current`, or `current` if at the start.
pub fn prev_actionable_index(current: usize, items: &[MenuItem]) -> usize {
    if current == 0 {
        return current;
    }

    let mut idx = current - 1;
    loop {
        if !items[idx].is_separator {
            return idx;
        }
        if idx == 0 {
            return current;
        }
        idx -= 1;
    }
}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let manager = self.tab_manager.read(cx);
        let active_id = manager.active_id();
        let drop_target_index = self.drop_target_index;

        let tab_data: Vec<_> = manager
            .documents()
            .iter()
            .map(|doc| {
                (
                    doc.meta_snapshot(cx),
                    doc.change_summary(cx),
                    doc.tab_tooltip(cx),
                    doc.tab_group(cx),
                )
            })
            .collect();

        // Bring a newly activated tab into view. Done here rather than on the
        // activation event so it also covers tabs opened while the bar was
        // busy elsewhere (the palette, a restored session).
        if active_id != self.last_active_id {
            self.last_active_id = active_id;
            if let Some(index) = tab_data
                .iter()
                .position(|(meta, ..)| Some(meta.id) == active_id)
            {
                self.scroll_handle.scroll_to_item(index);
            }
        }

        let mut tabs: Vec<AnyElement> = Vec::with_capacity(tab_data.len());
        let mut previous_group: Option<TabGroupKey> = None;
        for (idx, (meta, change_summary, tooltip, group)) in tab_data.into_iter().enumerate() {
            let group = group.map(|database| TabGroupKey {
                connection_id: meta.connection_id,
                database,
            });
            let starts_group = group.is_some() && group != previous_group;
            let band = TabGroupBand::new(group.as_ref(), starts_group, cx.theme());
            previous_group = group;
            tabs.push(
                self.render_tab(
                    meta,
                    change_summary,
                    tooltip,
                    idx,
                    active_id,
                    drop_target_index,
                    band,
                    cx,
                )
                .into_any_element(),
            );
        }

        let new_tab_btn = self.render_new_tab_button(cx).into_any_element();

        document_tab_bar(cx).id("tab-bar").w_full().min_w_0().child(
            div()
                .id("document-tab-list")
                .role(Role::TabList)
                .flex()
                .min_w_0()
                .items_end()
                .overflow_x_scroll()
                .track_scroll(&self.scroll_handle)
                .gap(TabMetrics::DOCUMENT_BAR_GAP)
                // A drag that ends outside a tab leaves the insertion marker
                // behind; clearing it here covers every release.
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.drop_target_index.take().is_some() {
                            cx.notify();
                        }
                    }),
                )
                .children(tabs)
                .child(new_tab_btn),
        )
    }
}

impl TabBar {
    #[allow(clippy::too_many_arguments)]
    fn render_tab(
        &self,
        meta: DocumentMetaSnapshot,
        change_summary: Option<String>,
        tooltip: Option<SharedString>,
        idx: usize,
        active_id: Option<DocumentId>,
        drop_target_index: Option<usize>,
        band: TabGroupBand,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = meta.id;
        let is_active = active_id == Some(id);
        let is_executing = meta.state == DocumentState::Executing;
        let is_dirty = meta.state == DocumentState::Modified;
        let is_drop_target = drop_target_index == Some(idx);

        let title = meta.title.clone();

        let tab_manager = self.tab_manager.clone();

        let icon = tab_icon(meta.icon);

        let center_x = self.active_tab_center_x.clone();

        let theme = cx.theme();
        let tint = ChromeColors::tint(theme);
        let icon_color = if is_active {
            tint
        } else {
            theme.muted_foreground
        };
        let title_element = if is_active {
            document_tab_title(title, true, cx)
        } else {
            document_tab_title(title, false, cx).font_weight(FontWeight::NORMAL)
        };
        let hover_group: SharedString = format!("tab-group-{}", id.0).into();
        let drag_label: SharedString = meta.title.clone().into();
        let band_text_color = theme.background;

        let band = div()
            .h(TAB_GROUP_BAND)
            .w_full()
            .px(Spacing::SM)
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded_t(Radii::SM)
            .when_some(band.color, |el, color| el.bg(color))
            .when_some(band.label, |el, label| {
                el.child(
                    div().flex_1().min_w_0().truncate().child(
                        Text::caption(label)
                            .font_size(FontSizes::XS)
                            .color(band_text_color),
                    ),
                )
            });

        let chip = document_tab(
            ElementId::Name(format!("tab-{}", id.0).into()),
            is_active,
            cx,
        )
        .group(hover_group.clone())
        .debug_selector(|| format!("tab-{}", id.0))
        .role(Role::Tab)
        .aria_selected(is_active)
        .w_full()
        .when(is_active, |el| {
            el.child(
                canvas(
                    move |bounds: Bounds<Pixels>, _, _| {
                        center_x.set(bounds.center().x);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
        })
        .when(is_drop_target, |el| el.border_l_2().border_color(tint))
        .when_some(tooltip, |el, tooltip| {
            el.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
        // Click to activate
        .on_click({
            let tab_manager = tab_manager.clone();
            cx.listener(move |_this, _event, _window, cx| {
                tab_manager.update(cx, |mgr, cx| {
                    mgr.activate(id, cx);
                });
            })
        })
        // Middle-click to close: routed through the workspace, like every
        // other close gesture, so pending edits are persisted first.
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _event, _window, cx| {
                this.request_close(id, cx);
            }),
        )
        // Right-click for context menu
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                this.commit_active_pending_input(cx);
                this.context_menu = Some(TabContextMenu {
                    tab_id: id,
                    tab_index: idx,
                    position_x: event.position.x,
                    selected_index: 0,
                });
                cx.notify();
            }),
        )
        .child(Icon::new(icon).size(TabMetrics::ICON).color(icon_color))
        .child(div().flex_1().min_w_0().truncate().child(title_element))
        // Dirty indicator: a tint diamond when the document has unsaved
        // changes. Shows the change summary in a tooltip on hover.
        .when(is_dirty, |el| {
            let tooltip_text: SharedString = change_summary
                .unwrap_or_else(|| dbflux_i18n::t!("document.tabs.unsaved_changes"))
                .into();

            el.child(
                div()
                    .id(ElementId::Name(format!("dirty-dot-{}", id.0).into()))
                    .flex_shrink_0()
                    .child(StatusIndicator::new(Status::Busy).compact())
                    .tooltip(move |window, cx| {
                        Tooltip::new(tooltip_text.clone()).build(window, cx)
                    }),
            )
        })
        // Spinner or close button. Inactive tabs show the close button only
        // while hovered.
        .child(self.render_tab_action(id, is_executing, is_active, hover_group, cx));

        div()
            .id(ElementId::Name(format!("tab-column-{}", id.0).into()))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_w(TAB_MIN_WIDTH)
            .max_w(if is_active {
                TAB_ACTIVE_MAX_WIDTH
            } else {
                TAB_MAX_WIDTH
            })
            // Drag to reorder. The payload carries the index the tab started
            // at, because by drop time the pointer only tells us where it
            // landed.
            .on_drag(
                TabDrag {
                    index: idx,
                    label: drag_label,
                },
                |drag, _, _, cx| {
                    cx.new(|_| TabDragPreview {
                        label: drag.label.clone(),
                    })
                },
            )
            .drag_over::<TabDrag>({
                let tab_bar = cx.entity().clone();
                move |style, _, _, cx| {
                    tab_bar.update(cx, |this, cx| {
                        if this.drop_target_index != Some(idx) {
                            this.drop_target_index = Some(idx);
                            cx.notify();
                        }
                    });
                    style
                }
            })
            .on_drop(cx.listener(move |this, drag: &TabDrag, _window, cx| {
                this.drop_target_index = None;
                this.tab_manager.update(cx, |manager, cx| {
                    manager.move_tab(drag.index, idx, cx);
                });
                cx.notify();
            }))
            .child(band)
            .child(chip)
    }

    fn render_tab_action(
        &self,
        id: DocumentId,
        is_executing: bool,
        is_active: bool,
        hover_group: SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if is_executing {
            return Icon::new(AppIcon::Loader)
                .size(TabMetrics::CLOSE_ICON)
                .color(ChromeColors::tint(cx.theme()))
                .into_any_element();
        }

        let hover = cx.theme().secondary;

        div()
            .id(ElementId::Name(format!("tab-close-{}", id.0).into()))
            .debug_selector(|| format!("tab-close-{}", id.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(move |el| el.bg(hover))
            .when(!is_active, |el| {
                el.invisible()
                    .group_hover(hover_group, |style| style.visible())
            })
            .child(
                Icon::new(AppIcon::CircleX)
                    .size(TabMetrics::CLOSE_ICON)
                    .muted(),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _event, _window, cx| {
                cx.stop_propagation();
                this.request_close(id, cx);
            }))
            .into_any_element()
    }

    fn render_new_tab_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .ml(TabMetrics::NEW_TAB_MARGIN_LEFT)
            .flex()
            .flex_shrink_0()
            .items_center()
            .child(
                Button::new("new-tab-btn", dbflux_i18n::t!("document.tabs.new"))
                    .ghost()
                    .icon(AppIcon::Plus)
                    .icon_size(TabMetrics::ICON)
                    .icon_only()
                    .on_click(cx.listener(|_this, _event, _window, cx| {
                        cx.emit(TabBarEvent::NewTabRequested);
                    })),
            )
    }
}

impl EventEmitter<TabBarEvent> for TabBar {}

#[derive(Clone, Debug)]
pub enum TabBarEvent {
    NewTabRequested,
    CloseTab(DocumentId),
    CloseOtherTabs(DocumentId),
    CloseAllTabs,
    CloseTabsToLeft(DocumentId),
    CloseTabsToRight(DocumentId),
}

#[cfg(test)]
mod tests {
    use super::{
        TAB_MENU_CLOSE, TAB_MENU_CLOSE_ALL, TAB_MENU_CLOSE_LEFT, TAB_MENU_CLOSE_OTHERS,
        TAB_MENU_CLOSE_RIGHT, TAB_MENU_SEPARATOR, TabBar, TabBarEvent, next_actionable_index,
        prev_actionable_index,
    };
    use crate::code::CodeDocument;
    use crate::tab_manager::Tab;
    use crate::tab_manager::TabManager;
    use crate::types::DocumentId;
    use dbflux_components::composites::document_tab_title;
    use dbflux_components::primitives::TextVariant;
    use dbflux_components::theme;
    use dbflux_components::typography::AppFonts;
    use dbflux_core::QueryLanguage;
    use dbflux_storage::bootstrap::StorageRuntime;
    use dbflux_ui_base::AppStateEntity;
    use dbflux_ui_base::toast::{ToastGlobal, ToastHost};
    use gpui::{
        AccessibilityFrame, AppContext as _, FontWeight, FrameObserver, Role, TestAppContext,
        VisualTestContext,
    };
    use gpui_component::theme::Theme;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    #[test]
    fn build_tab_menu_items_returns_correct_structure() {
        let items = TabBar::build_tab_menu_items();

        assert_eq!(items.len(), 6);
        assert_eq!(
            items[TAB_MENU_CLOSE].label.as_ref(),
            dbflux_i18n::t!("document.tabs.menu.close", locale = "en")
        );
        assert_eq!(
            items[TAB_MENU_CLOSE_OTHERS].label.as_ref(),
            dbflux_i18n::t!("document.tabs.menu.close_others", locale = "en")
        );
        assert_eq!(
            items[TAB_MENU_CLOSE_ALL].label.as_ref(),
            dbflux_i18n::t!("document.tabs.menu.close_all", locale = "en")
        );
        assert!(items[TAB_MENU_SEPARATOR].is_separator);
        assert_eq!(
            items[TAB_MENU_CLOSE_LEFT].label.as_ref(),
            dbflux_i18n::t!("document.tabs.menu.close_left", locale = "en")
        );
        assert_eq!(
            items[TAB_MENU_CLOSE_RIGHT].label.as_ref(),
            dbflux_i18n::t!("document.tabs.menu.close_right", locale = "en")
        );
    }

    #[test]
    fn tab_menu_keys_resolve_in_both_locales() {
        let keys = [
            "document.tabs.menu.close",
            "document.tabs.menu.close_others",
            "document.tabs.menu.close_all",
            "document.tabs.menu.close_left",
            "document.tabs.menu.close_right",
            "document.tabs.unsaved_changes",
        ];

        for key in keys {
            for locale in ["en", "es"] {
                let value = dbflux_i18n::t!(key, locale = locale);
                assert!(!value.is_empty(), "{locale}.{key} resolved empty");
                assert_ne!(value, key, "{locale}.{key} resolved to the raw key");
                assert_ne!(
                    value,
                    format!("{locale}.{key}"),
                    "{locale}.{key} resolved to the missing-translation sentinel"
                );
            }
        }
    }

    #[test]
    fn tab_menu_close_differs_between_locales() {
        let english = dbflux_i18n::t!("document.tabs.menu.close", locale = "en");
        let spanish = dbflux_i18n::t!("document.tabs.menu.close", locale = "es");

        assert_ne!(english, spanish);
    }

    #[test]
    fn build_tab_menu_items_have_icons() {
        let items = TabBar::build_tab_menu_items();

        for (idx, item) in items.iter().enumerate() {
            if item.is_separator {
                assert!(item.icon.is_none(), "separator should have no icon");
            } else {
                assert!(item.icon.is_some(), "item {} should have an icon", idx);
            }
        }
    }

    #[test]
    fn no_tab_menu_items_are_danger_or_submenu() {
        let items = TabBar::build_tab_menu_items();

        for item in &items {
            assert!(!item.is_danger);
            assert!(!item.has_submenu);
        }
    }

    #[test]
    fn next_actionable_skips_separator() {
        let items = TabBar::build_tab_menu_items();

        // 0 -> 1 -> 2 -> 4 (skip separator at 3) -> 5
        assert_eq!(next_actionable_index(0, &items), 1);
        assert_eq!(next_actionable_index(1, &items), 2);
        assert_eq!(next_actionable_index(2, &items), 4);
        assert_eq!(next_actionable_index(4, &items), 5);
    }

    #[test]
    fn next_actionable_stays_at_end() {
        let items = TabBar::build_tab_menu_items();
        assert_eq!(next_actionable_index(5, &items), 5);
    }

    #[test]
    fn prev_actionable_skips_separator() {
        let items = TabBar::build_tab_menu_items();

        // 5 -> 4 -> 2 (skip separator at 3) -> 1 -> 0
        assert_eq!(prev_actionable_index(5, &items), 4);
        assert_eq!(prev_actionable_index(4, &items), 2);
        assert_eq!(prev_actionable_index(2, &items), 1);
        assert_eq!(prev_actionable_index(1, &items), 0);
    }

    #[test]
    fn prev_actionable_stays_at_start() {
        let items = TabBar::build_tab_menu_items();
        assert_eq!(prev_actionable_index(0, &items), 0);
    }

    #[gpui::test]
    fn tab_titles_are_strong_when_active_and_muted_otherwise(cx: &mut TestAppContext) {
        cx.update(theme::init);

        let (active, inactive) = cx.update(|cx| {
            (
                document_tab_title("query.sql", true, cx).inspect(),
                document_tab_title("table/users", false, cx).inspect(),
            )
        });

        for inspection in [active, inactive] {
            assert_eq!(inspection.variant, TextVariant::Body);
            assert_eq!(inspection.family, AppFonts::INTERFACE);
        }

        assert!(active.has_custom_color_override);
        assert_eq!(active.weight_override, Some(FontWeight::SEMIBOLD));
        assert!(inactive.uses_muted_foreground_override);
    }

    /// The close button, a middle-click, and the context menu all emit the same
    /// `TabBarEvent::CloseTab`, so the workspace applies one close policy to
    /// every gesture instead of the bar removing tabs directly.
    #[gpui::test]
    fn every_close_gesture_requests_close_through_one_event(cx: &mut TestAppContext) {
        cx.update(theme::init);

        let manager = cx.update(|cx| cx.new(|_| TabManager::new()));
        let bar = cx.update(|cx| cx.new(|cx| TabBar::new(manager, cx)));
        let id = DocumentId::new();

        let events: Rc<RefCell<Vec<TabBarEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&bar, move |_, event: &TabBarEvent, _| {
                sink.borrow_mut().push(event.clone());
            })
            .detach();
        });

        cx.update(|cx| {
            bar.update(cx, |bar, cx| bar.request_close(id, cx));
        });
        cx.run_until_parked();

        let recorded = events.borrow().clone();
        assert!(
            matches!(recorded.as_slice(), [TabBarEvent::CloseTab(got)] if *got == id),
            "every close gesture shares one close request, got {recorded:?}"
        );
    }

    /// Keeps the latest rendered accessibility frame of the window it observes.
    #[derive(Default)]
    struct FrameCapture(Mutex<Option<AccessibilityFrame>>);

    impl FrameObserver for FrameCapture {
        fn accessibility_updated(&self, frame: &AccessibilityFrame) {
            *self.0.lock().expect("frame capture lock") = Some(frame.clone());
        }
    }

    /// Document tabs are exposed as tabs inside one tab list, and only the
    /// active tab reports itself selected.
    #[gpui::test]
    fn tabs_are_exposed_as_selectable_tabs_in_a_tab_list(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(theme::init);
        cx.update(|cx| {
            let host = cx.new(|_| ToastHost::new());
            cx.set_global(ToastGlobal { host });
        });

        let app_state = cx.update(|cx| {
            cx.new(|_| {
                AppStateEntity::new_with_storage_runtime(
                    StorageRuntime::in_memory().expect("isolated storage runtime"),
                )
                .expect("test storage setup")
            })
        });
        let manager = cx.update(|cx| cx.new(|_| TabManager::new()));

        let capture = Arc::new(FrameCapture::default());
        let (_bar, window) = cx.add_window_view({
            let manager = manager.clone();
            let capture = capture.clone();
            move |window, cx| {
                window.observe_frames(&capture);
                TabBar::new(manager, cx)
            }
        });

        let open_tab = |window: &mut VisualTestContext| {
            let app_state = app_state.clone();
            let manager = manager.clone();
            window.update(|window, cx| {
                let document = cx.new(|cx| {
                    CodeDocument::new_with_language(app_state, None, QueryLanguage::Sql, window, cx)
                });
                let document_id = document.read(cx).id();
                let pane = CodeDocument::into_pane(document, cx);
                manager.update(cx, |manager, cx| {
                    manager.open(Tab::Pane(Box::new(pane)), cx)
                });
                document_id
            })
        };
        let background = open_tab(window);
        let active = open_tab(window);

        window.update(|window, _| window.refresh());
        window.run_until_parked();

        let frame = capture
            .0
            .lock()
            .expect("frame capture lock")
            .clone()
            .expect("the window rendered a frame");
        let node = |id: &str| {
            frame
                .nodes()
                .find(|(_, node)| node.id() == id)
                .and_then(|(_, node)| frame.accessibility_node(node))
                .unwrap_or_else(|| panic!("no accessible node {id}"))
        };

        assert_eq!(node("document-tab-list").role(), Role::TabList);

        for (id, selected) in [(active, true), (background, false)] {
            let tab = node(&format!("tab-{}", id.0));
            assert_eq!(tab.role(), Role::Tab);
            assert_eq!(tab.is_selected(), Some(selected), "tab {id:?}");
        }
    }
}

/// The icon a tab shows for its document kind, matching the icon the sidebar
/// gives the same object (a collection's box, a key-value database's key).
fn tab_icon(icon: super::types::DocumentIcon) -> AppIcon {
    use super::types::DocumentIcon;

    match icon {
        DocumentIcon::Sql => AppIcon::Code,
        DocumentIcon::Table => AppIcon::Table,
        DocumentIcon::Redis => AppIcon::KeyRound,
        DocumentIcon::RedisKey => AppIcon::Hash,
        DocumentIcon::Terminal => AppIcon::SquareTerminal,
        DocumentIcon::Mongo => AppIcon::Database,
        DocumentIcon::Collection => AppIcon::Box,
        DocumentIcon::Script => AppIcon::ScrollText,
        DocumentIcon::Audit => AppIcon::ScrollText,
        DocumentIcon::SchemaViz => AppIcon::Layers,
        DocumentIcon::Chart => AppIcon::ChartSpline,
        DocumentIcon::Dashboard => AppIcon::ChartSpline,
        DocumentIcon::Buckets => AppIcon::Box,
        DocumentIcon::ObjectBrowser => AppIcon::Folder,
        DocumentIcon::DumpAnalysis => AppIcon::HardDrive,
        DocumentIcon::McpApprovals => AppIcon::Bot,
        DocumentIcon::Migrate => AppIcon::ArrowUpDown,
    }
}

#[cfg(test)]
mod tab_icon_tests {
    use super::tab_icon;
    use crate::types::DocumentIcon;
    use dbflux_components::icons::AppIcon;

    #[test]
    fn collection_and_key_value_tabs_use_their_sidebar_icons() {
        assert_eq!(tab_icon(DocumentIcon::Collection), AppIcon::Box);
        assert_eq!(tab_icon(DocumentIcon::Redis), AppIcon::KeyRound);
        assert_eq!(tab_icon(DocumentIcon::SchemaViz), AppIcon::Layers);
    }
}

#[cfg(test)]
mod group_band_tests {
    use super::{TAB_MENU_EDGE_GAP, clamp_tab_menu_left, window_title};
    use gpui::px;

    #[test]
    fn menu_opens_at_the_click_when_it_fits() {
        assert_eq!(
            clamp_tab_menu_left(px(100.0), px(220.0), px(1200.0)),
            px(100.0)
        );
    }

    #[test]
    fn menu_is_pulled_left_of_the_window_edge() {
        let left = clamp_tab_menu_left(px(1150.0), px(220.0), px(1200.0));
        assert_eq!(left, px(1200.0) - px(220.0) - TAB_MENU_EDGE_GAP);
    }

    #[test]
    fn menu_stays_attached_to_the_left_edge_in_a_narrow_window() {
        assert_eq!(
            clamp_tab_menu_left(px(50.0), px(220.0), px(150.0)),
            TAB_MENU_EDGE_GAP
        );
    }

    #[test]
    fn window_title_leads_with_the_document_then_its_database() {
        assert_eq!(
            window_title(Some(("flags", Some("monixa-test"))), "DBFlux"),
            "flags - monixa-test - DBFlux"
        );
        assert_eq!(
            window_title(Some(("query.sql", None)), "DBFlux"),
            "query.sql - DBFlux"
        );
        assert_eq!(window_title(None, "DBFlux"), "DBFlux");
    }
}
