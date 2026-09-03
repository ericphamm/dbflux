use std::cell::Cell;
use std::rc::Rc;

use super::tab_manager::TabManager;
use super::types::{DocumentId, DocumentMetaSnapshot, DocumentState};
use dbflux_components::composites::MenuItem;
use dbflux_components::icons::AppIcon;
use dbflux_components::primitives::{Icon, Text};
use dbflux_components::semantic::BannerColors as SemBannerColors;
use dbflux_components::tokens::FontSizes;
use dbflux_components::tokens::{Heights, Radii, Spacing};
use dbflux_components::typography::{MonoCaption, MonoMeta};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::tooltip::Tooltip;
use uuid::Uuid;

const TAB_BAR_HEIGHT: Pixels = Heights::TAB;

/// Full height of the bar, band included. The workspace anchors the tab
/// context menu just below it, so this has to be one value rather than a
/// literal repeated at the call site — the band changed the height once
/// already and left the menu overlapping the toolbar.
pub const TAB_BAR_TOTAL_HEIGHT: Pixels = px(52.0);

/// Width the tab context menu is assumed to take when deciding whether it
/// fits. The menu itself is min-width driven, so this is the floor plus room
/// for the longest label ("Close Tabs to the Right").
pub const TAB_MENU_WIDTH: Pixels = px(220.0);

/// Space kept between the menu and the window edge.
const TAB_MENU_EDGE_GAP: Pixels = Spacing::SM;

/// Narrowest a tab may get. Tabs never shrink past this, however many are
/// open — the strip scrolls instead, because a row of four-letter stumps
/// tells the user nothing about which table each tab holds.
const TAB_MIN_WIDTH: Pixels = px(140.0);

/// Widest an inactive tab gets before its title is ellipsized.
const TAB_MAX_WIDTH: Pixels = px(220.0);

/// Widest the active tab gets. Larger than the rest so the table you are
/// actually looking at shows its whole name.
const TAB_ACTIVE_MAX_WIDTH: Pixels = px(360.0);

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

/// Height of the band above each tab that names the database the tab
/// belongs to. Every tab gets the band so the bar keeps one height; tabs
/// without a database leave it blank.
const TAB_GROUP_BAND: Pixels = Spacing::LG;

/// What makes two neighbouring tabs share a band: same connection, same
/// database.
#[derive(Clone, PartialEq, Eq)]
struct TabGroupKey {
    connection_id: Option<Uuid>,
    database: String,
}

impl TabGroupKey {
    fn for_meta(meta: &DocumentMetaSnapshot) -> Option<Self> {
        meta.group.clone().map(|database| Self {
            connection_id: meta.connection_id,
            database,
        })
    }

    /// A colour that stays the same for this database for the whole session,
    /// drawn from the theme's chart palette so it fits either theme. Hashing
    /// rather than counting groups keeps a database's colour stable when tabs
    /// open and close around it.
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
                .map(|group| SharedString::from(group.database.clone())),
        }
    }
}

/// What the tab's tooltip says: the band's database and the title together,
/// so two tabs named `flags` in different databases stay tellable apart on
/// hover even though the tab itself shows only the object name.
fn tab_tooltip(title: &str, group: Option<&str>) -> String {
    match group {
        Some(group) => format!("{group}.{title}"),
        None => title.to_string(),
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
            .map(|doc| (doc.meta_snapshot(cx), doc.change_summary(cx)))
            .collect();

        // Bring a newly activated tab into view. Done here rather than on the
        // activation event so it also covers tabs opened while the bar was
        // busy elsewhere (the palette, a restored session).
        if active_id != self.last_active_id {
            self.last_active_id = active_id;
            if let Some(index) = tab_data
                .iter()
                .position(|(meta, _)| Some(meta.id) == active_id)
            {
                self.scroll_handle.scroll_to_item(index);
            }
        }

        let mut tabs: Vec<AnyElement> = Vec::with_capacity(tab_data.len());
        let mut previous_group: Option<TabGroupKey> = None;
        for (idx, (meta, change_summary)) in tab_data.into_iter().enumerate() {
            let group = TabGroupKey::for_meta(&meta);
            let starts_group = group.is_some() && group != previous_group;
            let band = TabGroupBand::new(group.as_ref(), starts_group, cx.theme());
            previous_group = group;
            tabs.push(
                self.render_tab(
                    meta,
                    change_summary,
                    idx,
                    active_id,
                    drop_target_index,
                    band,
                    cx,
                )
                .into_any_element(),
            );
        }

        let tab_bar_bg = cx.theme().tab_bar;
        let border_color = cx.theme().border;
        let new_tab_btn = self.render_new_tab_button(cx).into_any_element();

        div()
            .id("tab-bar")
            .h(TAB_BAR_HEIGHT + TAB_GROUP_BAND)
            .w_full()
            .flex()
            .items_center()
            .bg(tab_bar_bg)
            .border_b_1()
            .border_color(border_color)
            .child(
                div()
                    .id("tab-strip")
                    // A drag that ends outside a tab leaves the insertion
                    // marker behind; clearing it here covers every release.
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.drop_target_index.take().is_some() {
                                cx.notify();
                            }
                        }),
                    )
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .overflow_x_scroll()
                    .track_scroll(&self.scroll_handle)
                    .gap_px()
                    .children(tabs)
                    .child(new_tab_btn),
            )
    }
}

impl TabBar {
    fn tab_title_text(
        title: SharedString,
        is_active: bool,
        theme: &gpui_component::Theme,
    ) -> MonoMeta {
        MonoMeta::new(title).color(if is_active {
            theme.foreground
        } else {
            theme.muted_foreground
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tab(
        &self,
        meta: DocumentMetaSnapshot,
        change_summary: Option<String>,
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
        let tooltip: SharedString = tab_tooltip(&meta.title, meta.group.as_deref()).into();
        let band_text_color = cx.theme().background;

        let tab_manager = self.tab_manager.clone();

        let icon = match meta.icon {
            super::types::DocumentIcon::Sql => AppIcon::Code,
            super::types::DocumentIcon::Table => AppIcon::Table,
            super::types::DocumentIcon::Redis => AppIcon::Database,
            super::types::DocumentIcon::RedisKey => AppIcon::Hash,
            super::types::DocumentIcon::Terminal => AppIcon::SquareTerminal,
            super::types::DocumentIcon::Mongo => AppIcon::Database,
            super::types::DocumentIcon::Collection => AppIcon::Folder,
            super::types::DocumentIcon::Script => AppIcon::ScrollText,
            super::types::DocumentIcon::Audit => AppIcon::ScrollText,
            super::types::DocumentIcon::Chart => AppIcon::ChartSpline,
            super::types::DocumentIcon::Dashboard => AppIcon::ChartSpline,
            super::types::DocumentIcon::Buckets => AppIcon::Box,
            super::types::DocumentIcon::ObjectBrowser => AppIcon::Folder,
            super::types::DocumentIcon::DumpAnalysis => AppIcon::HardDrive,
        };

        let center_x = self.active_tab_center_x.clone();

        let band = div()
            .h(TAB_GROUP_BAND)
            .w_full()
            .px(Spacing::SM)
            .flex()
            .items_center()
            .overflow_hidden()
            .when_some(band.color, |el, color| el.bg(color))
            .when_some(band.label, |el, label| {
                el.child(
                    div()
                        .flex_1()
                        .truncate()
                        .child(MonoCaption::new(label).color(band_text_color)),
                )
            });

        let row = div()
            .relative()
            .flex_1()
            .w_full()
            .px(Spacing::MD)
            .flex()
            .items_center()
            .gap(Spacing::SM)
            .when(is_active, |el| {
                let stripe_color = cx.theme().primary;
                el.bg(cx.theme().tab_bar)
                    .child(
                        // Active-tab indicator: 1 px stripe at the bottom edge.
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(Heights::TAB_STRIPE)
                            .bg(stripe_color),
                    )
                    .child(
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
            .when(!is_active, |el| el.hover(|el| el.bg(cx.theme().secondary)))
            // Icon
            .child(Icon::new(icon).size(Heights::ICON_SM).color(if is_active {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            }))
            // Title. The band above carries the database, so this is the
            // object name alone; the tooltip restores the qualified form.
            .child(
                div()
                    .id(ElementId::Name(format!("tab-title-{}", id.0).into()))
                    .flex_1()
                    .truncate()
                    .child(Self::tab_title_text(title.into(), is_active, cx.theme()))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
            )
            // Dirty indicator: amber dot when the document has unsaved changes.
            // Shows the change summary in a tooltip on hover.
            .when(is_dirty, |el| {
                let dot_color = SemBannerColors::for_current(cx).warning_bg;
                let tooltip_text: SharedString = change_summary
                    .unwrap_or_else(|| dbflux_i18n::t!("document.tabs.unsaved_changes"))
                    .into();

                el.child(
                    div()
                        .id(ElementId::Name(format!("dirty-dot-{}", id.0).into()))
                        .w(Spacing::XXS)
                        .h(Spacing::XXS)
                        .rounded_full()
                        .bg(dot_color)
                        .flex_shrink_0()
                        .tooltip(move |window, cx| {
                            Tooltip::new(tooltip_text.clone()).build(window, cx)
                        }),
                )
            })
            // Spinner or close button
            .child(self.render_tab_action(id, is_executing, cx));

        div()
            .id(ElementId::Name(format!("tab-{}", id.0).into()))
            .relative()
            .h_full()
            .min_w(TAB_MIN_WIDTH)
            .max_w(if is_active {
                TAB_ACTIVE_MAX_WIDTH
            } else {
                TAB_MAX_WIDTH
            })
            // Without this the row of tabs divides the available width between
            // itself and ignores the minimum, which is what turned a dozen
            // open tables into a row of stumps.
            .flex_shrink_0()
            .flex()
            .flex_col()
            .cursor_pointer()
            .when(is_drop_target, |el| {
                el.border_l_2().border_color(cx.theme().accent)
            })
            // Drag to reorder. The payload carries the index the tab started
            // at, because by drop time the pointer only tells us where it
            // landed.
            .on_drag(
                TabDrag {
                    index: idx,
                    label: meta.title.clone().into(),
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
            // Click to activate
            .on_click({
                let tab_manager = tab_manager.clone();
                cx.listener(move |_this, _event, _window, cx| {
                    tab_manager.update(cx, |mgr, cx| {
                        mgr.activate(id, cx);
                    });
                })
            })
            // Middle-click to close
            .on_mouse_down(MouseButton::Middle, {
                let tab_manager = tab_manager.clone();
                cx.listener(move |_this, _event, _window, cx| {
                    tab_manager.update(cx, |mgr, cx| {
                        mgr.close(id, cx);
                    });
                })
            })
            // Right-click for context menu
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.context_menu = Some(TabContextMenu {
                        tab_id: id,
                        tab_index: idx,
                        position_x: event.position.x,
                        selected_index: 0,
                    });
                    cx.notify();
                }),
            )
            .child(band)
            .child(row)
    }

    fn render_tab_action(
        &self,
        id: DocumentId,
        is_executing: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let accent = cx.theme().accent;
        let secondary = cx.theme().secondary;
        let muted_fg = cx.theme().muted_foreground;

        div()
            .w(Heights::ICON_SM)
            .h(Heights::ICON_SM)
            .flex()
            .items_center()
            .justify_center()
            .rounded(Radii::SM)
            .child(if is_executing {
                Icon::new(AppIcon::Loader)
                    .size(px(12.0)) // guardrail-allow: 12px icon size, no ICON_XS token
                    .color(accent)
                    .into_any_element()
            } else {
                div()
                    .id(ElementId::Name(format!("tab-close-{}", id.0).into()))
                    .w(Heights::ICON_SM)
                    .h(Heights::ICON_SM)
                    .rounded(Radii::SM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(move |el| el.bg(secondary))
                    .child(Icon::new(AppIcon::X).size(px(12.0)).color(muted_fg)) // guardrail-allow: 12px icon size, no ICON_XS token
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event, _window, cx| {
                            cx.stop_propagation();
                            this.tab_manager.update(cx, |mgr, cx| {
                                mgr.close(id, cx);
                            });
                        }),
                    )
                    .into_any_element()
            })
    }

    fn render_new_tab_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("new-tab-btn")
            .w(px(32.0)) // guardrail-allow: new-tab button width, not a toolbar height token
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|el| el.bg(cx.theme().secondary))
            .child(Icon::new(AppIcon::Plus).size(px(14.0)).muted())
            .on_click(cx.listener(|_this, _event, _window, cx| {
                cx.emit(TabBarEvent::NewTabRequested);
            }))
    }
}

impl EventEmitter<TabBarEvent> for TabBar {}

#[cfg(test)]
mod group_band_tests {
    use super::{
        TAB_BAR_HEIGHT, TAB_BAR_TOTAL_HEIGHT, TAB_GROUP_BAND, TAB_MENU_EDGE_GAP,
        clamp_tab_menu_left, tab_tooltip,
    };
    use gpui::px;

    #[test]
    fn total_height_covers_the_bar_and_its_band() {
        assert_eq!(TAB_BAR_TOTAL_HEIGHT, TAB_BAR_HEIGHT + TAB_GROUP_BAND);
    }

    #[test]
    fn menu_opens_at_the_click_when_it_fits() {
        assert_eq!(
            clamp_tab_menu_left(px(300.0), px(220.0), px(1200.0)),
            px(300.0)
        );
    }

    #[test]
    fn menu_is_pulled_left_of_the_window_edge() {
        // Right-clicking the last tab of a 1200px window: 1100 + 220 would
        // put half the menu outside.
        assert_eq!(
            clamp_tab_menu_left(px(1100.0), px(220.0), px(1200.0)),
            px(1200.0) - px(220.0) - TAB_MENU_EDGE_GAP
        );
    }

    #[test]
    fn menu_stays_attached_to_the_left_edge_in_a_narrow_window() {
        assert_eq!(
            clamp_tab_menu_left(px(80.0), px(220.0), px(200.0)),
            TAB_MENU_EDGE_GAP
        );
    }

    #[test]
    fn window_title_leads_with_the_document_then_its_database() {
        assert_eq!(
            super::window_title(Some(("flags", Some("monixa-test"))), "DBFlux"),
            "flags - monixa-test - DBFlux"
        );
    }

    #[test]
    fn window_title_falls_back_to_the_product_alone() {
        assert_eq!(
            super::window_title(Some(("query.sql", None)), "DBFlux"),
            "query.sql - DBFlux"
        );
        assert_eq!(super::window_title(None, "DBFlux"), "DBFlux");
    }

    #[test]
    fn tooltip_restores_the_qualified_name() {
        assert_eq!(
            tab_tooltip("flags", Some("monixa-test")),
            "monixa-test.flags"
        );
    }

    #[test]
    fn tooltip_is_the_bare_title_without_a_group() {
        assert_eq!(tab_tooltip("query.sql", None), "query.sql");
    }
}

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
        TAB_MENU_CLOSE_RIGHT, TAB_MENU_SEPARATOR, TabBar, next_actionable_index,
        prev_actionable_index,
    };
    use dbflux_components::theme;
    use dbflux_components::tokens::FontSizes;
    use dbflux_components::typography::AppFonts;
    use gpui::TestAppContext;
    use gpui_component::theme::Theme;

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
    fn tab_titles_use_mono_meta_role(cx: &mut TestAppContext) {
        cx.update(theme::init);

        let theme = cx.update(|cx| Theme::global(cx).clone());

        let active = TabBar::tab_title_text("query.sql".into(), true, &theme).inspect();
        let inactive = TabBar::tab_title_text("table/users".into(), false, &theme).inspect();

        for inspection in [active, inactive] {
            assert_eq!(inspection.family, Some(AppFonts::MONO));
            assert_eq!(inspection.fallbacks, &[AppFonts::MONO_FALLBACK]);
            assert_eq!(inspection.size_override, Some(FontSizes::SM));
            assert_eq!(inspection.weight_override, None);
            assert!(inspection.has_custom_color_override);
        }
    }
}
