use super::ContextMenuItem;
use dbflux_app::keymap::{Command, ContextId};
use dbflux_components::components::data_table::ContextMenuAction;
use dbflux_components::icons::AppIcon;
use gpui::SharedString;

/// The keyboard shortcut for a menu action, or `None` when the action has no
/// binding in the results grid.
///
/// Read from the keymap rather than spelled out here, so a rebinding cannot
/// leave the menu advertising a key that no longer works.
fn action_shortcut(action: ContextMenuAction) -> Option<SharedString> {
    let command = match action {
        ContextMenuAction::Copy => Command::ResultsCopyCell,
        ContextMenuAction::ViewValue => Command::ToggleValuePanel,
        ContextMenuAction::Edit => Command::Rename,
        ContextMenuAction::AddRow => Command::ResultsAddRow,
        ContextMenuAction::DeleteRow => Command::Delete,
        _ => return None,
    };

    dbflux_ui_base::keymap::default_keymap()
        .shortcut_for_command(ContextId::Results, command)
        .map(SharedString::from)
}

pub(super) fn build_context_menu_items(
    is_editable: bool,
    is_document_view: bool,
    has_row_target: bool,
    can_chart: bool,
    inspect_row_enabled: bool,
) -> Vec<ContextMenuItem> {
    let mut items = build_menu_items(
        is_editable,
        is_document_view,
        has_row_target,
        can_chart,
        inspect_row_enabled,
    );

    for item in &mut items {
        if let Some(action) = item.action {
            item.shortcut = action_shortcut(action);
        }
    }

    items
}

fn build_menu_items(
    is_editable: bool,
    is_document_view: bool,
    has_row_target: bool,
    can_chart: bool,
    inspect_row_enabled: bool,
) -> Vec<ContextMenuItem> {
    if is_document_view {
        let mut items = Vec::new();

        if has_row_target {
            items.extend([
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.copy").into(),
                    action: Some(ContextMenuAction::Copy),
                    icon: Some(AppIcon::Layers),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.view_document").into(),
                    action: Some(ContextMenuAction::EditInModal),
                    icon: Some(AppIcon::Maximize2),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
            ]);
        }

        if is_editable {
            if !items.is_empty() {
                items.push(ContextMenuItem {
                    label: "".into(),
                    action: None,
                    icon: None,
                    is_separator: true,
                    is_danger: false,
                    shortcut: None,
                });
            }

            items.push(ContextMenuItem {
                label: dbflux_i18n::t!("document.data.context_menu.item.add_document").into(),
                action: Some(ContextMenuAction::AddRow),
                icon: Some(AppIcon::Plus),
                is_separator: false,
                is_danger: false,
                shortcut: None,
            });

            if has_row_target {
                items.extend([
                    ContextMenuItem {
                        label: dbflux_i18n::t!(
                            "document.data.context_menu.item.duplicate_document"
                        )
                        .into(),
                        action: Some(ContextMenuAction::DuplicateRow),
                        icon: Some(AppIcon::Layers),
                        is_separator: false,
                        is_danger: false,
                        shortcut: None,
                    },
                    ContextMenuItem {
                        label: dbflux_i18n::t!("document.data.context_menu.item.delete_document")
                            .into(),
                        action: Some(ContextMenuAction::DeleteRow),
                        icon: Some(AppIcon::Delete),
                        is_separator: false,
                        is_danger: true,
                        shortcut: None,
                    },
                ]);
            }
        }

        return items;
    }

    let mut items = vec![ContextMenuItem {
        label: dbflux_i18n::t!("document.data.context_menu.item.copy").into(),
        action: Some(ContextMenuAction::Copy),
        icon: Some(AppIcon::Layers),
        is_separator: false,
        is_danger: false,
        shortcut: None,
    }];

    // Offered for read-only results too — the panel is how a long JSON or XML
    // value gets read at all, whether or not it can be changed. It still needs
    // a row under the cursor: without one there is no value to show.
    if has_row_target {
        items.push(ContextMenuItem {
            label: dbflux_i18n::t!("document.data.context_menu.item.view_value").into(),
            action: Some(ContextMenuAction::ViewValue),
            icon: Some(AppIcon::Maximize2),
            is_separator: false,
            is_danger: false,
            shortcut: None,
        });
    }

    if is_editable {
        if has_row_target {
            items.extend([
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.paste").into(),
                    action: Some(ContextMenuAction::Paste),
                    icon: Some(AppIcon::Download),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.edit").into(),
                    action: Some(ContextMenuAction::Edit),
                    icon: Some(AppIcon::Pencil),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.edit_in_modal").into(),
                    action: Some(ContextMenuAction::EditInModal),
                    icon: Some(AppIcon::Maximize2),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: "".into(),
                    action: None,
                    icon: None,
                    is_separator: true,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.set_default").into(),
                    action: Some(ContextMenuAction::SetDefault),
                    icon: Some(AppIcon::RotateCcw),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.set_null").into(),
                    action: Some(ContextMenuAction::SetNull),
                    icon: Some(AppIcon::X),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: "".into(),
                    action: None,
                    icon: None,
                    is_separator: true,
                    is_danger: false,
                    shortcut: None,
                },
            ]);
        }

        items.push(ContextMenuItem {
            label: dbflux_i18n::t!("document.data.context_menu.item.add_row").into(),
            action: Some(ContextMenuAction::AddRow),
            icon: Some(AppIcon::Plus),
            is_separator: false,
            is_danger: false,
            shortcut: None,
        });

        if has_row_target {
            if inspect_row_enabled {
                items.push(ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.inspect_row").into(),
                    action: Some(ContextMenuAction::InspectRow),
                    icon: Some(AppIcon::Info),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                });
            }

            items.extend([
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.duplicate_row").into(),
                    action: Some(ContextMenuAction::DuplicateRow),
                    icon: Some(AppIcon::Layers),
                    is_separator: false,
                    is_danger: false,
                    shortcut: None,
                },
                ContextMenuItem {
                    label: dbflux_i18n::t!("document.data.context_menu.item.delete_row").into(),
                    action: Some(ContextMenuAction::DeleteRow),
                    icon: Some(AppIcon::Delete),
                    is_separator: false,
                    is_danger: true,
                    shortcut: None,
                },
            ]);
        }
    }

    if can_chart {
        items.push(ContextMenuItem {
            label: "".into(),
            action: None,
            icon: None,
            is_separator: true,
            is_danger: false,
            shortcut: None,
        });
        items.push(ContextMenuItem {
            label: dbflux_i18n::t!("document.data.context_menu.item.chart_this_query").into(),
            action: Some(ContextMenuAction::ChartThisQuery),
            icon: Some(AppIcon::ChartSpline),
            is_separator: false,
            is_danger: false,
            shortcut: None,
        });
    }

    items
}
