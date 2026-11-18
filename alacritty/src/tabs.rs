use std::mem;

use unicode_width::UnicodeWidthChar;

pub type TabId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAction {
    Next,
    Previous,
    Index(usize),
    Last,
}

pub struct Tabs<T> {
    pub active: T,
    pub active_index: usize,
    inactive: Vec<T>,
}

impl<T> Tabs<T> {
    pub fn new(active: T) -> Self {
        Self { active, active_index: 0, inactive: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.inactive.len() + 1
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        if index == self.active_index {
            Some(&self.active)
        } else {
            self.inactive.get(index - usize::from(index > self.active_index))
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index == self.active_index {
            Some(&mut self.active)
        } else {
            self.inactive.get_mut(index - usize::from(index > self.active_index))
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        (0..self.len()).map(|index| self.get(index).unwrap())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        let (before, after) = self.inactive.split_at_mut(self.active_index);
        before.iter_mut().chain(std::iter::once(&mut self.active)).chain(after.iter_mut())
    }

    pub fn push(&mut self, tab: T) {
        self.inactive.push(tab);
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.len() || index == self.active_index {
            return false;
        }

        let incoming_index = index - usize::from(index > self.active_index);
        let incoming = self.inactive.remove(incoming_index);
        let outgoing = mem::replace(&mut self.active, incoming);
        let outgoing_index = self.active_index - usize::from(self.active_index > index);
        self.inactive.insert(outgoing_index, outgoing);
        self.active_index = index;
        true
    }

    pub fn move_to(&mut self, from: usize, to: usize) -> bool {
        if from >= self.len() || to >= self.len() || from == to {
            return false;
        }
        if from == self.active_index {
            self.active_index = to;
            return true;
        }
        let inactive_index = from - usize::from(from > self.active_index);
        let tab = self.inactive.remove(inactive_index);
        if from < self.active_index && to >= self.active_index {
            self.active_index -= 1;
        } else if from > self.active_index && to <= self.active_index {
            self.active_index += 1;
        }
        let target_index = to - usize::from(to > self.active_index);
        self.inactive.insert(target_index, tab);
        true
    }

    pub fn remove(&mut self, index: usize) -> bool {
        self.take(index).is_some()
    }

    pub fn take(&mut self, index: usize) -> Option<T> {
        if index >= self.len() || self.len() == 1 {
            return None;
        }
        if index == self.active_index {
            let next = if index + 1 < self.len() { index + 1 } else { index - 1 };
            self.select(next);
        }
        let inactive_index = index - usize::from(index > self.active_index);
        let tab = self.inactive.remove(inactive_index);
        self.active_index -= usize::from(index < self.active_index);
        Some(tab)
    }

    pub fn target(&self, action: TabAction) -> usize {
        match action {
            TabAction::Next => (self.active_index + 1) % self.len(),
            TabAction::Previous => (self.active_index + self.len() - 1) % self.len(),
            TabAction::Index(index) => index,
            TabAction::Last => self.len() - 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabActivity {
    #[default]
    Idle,
    Running,
    Success,
    Failed,
}

#[derive(Default)]
pub struct TabBar {
    pub titles: Vec<String>,
    pub activities: Vec<TabActivity>,
    pub agent_states: Vec<Option<crate::agent::AgentState>>,
    pub agent_labels: Vec<String>,
    pub agent_unread: usize,
    pub agent_count: usize,
    pub agent_indicator: Option<crate::agent::AgentState>,
    pub groups: Vec<Option<String>>,
    pub group_filter: Option<String>,
    pub group_label: String,
    pub active: usize,
    pub hovered: Option<TabButton>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabButton {
    Select(usize),
    Close(usize),
    Create,
    Menu,
    Groups,
    Agents,
}

pub struct TabLayout {
    pub indices: Vec<usize>,
    pub tab_start: usize,
    pub first: usize,
    pub visible: usize,
    pub width: usize,
    pub button_start: usize,
}

impl TabBar {
    pub const ROWS: usize = 1;

    pub fn hover(&mut self, hit: Option<TabButton>) -> bool {
        let changed = self.hovered != hit;
        self.hovered = hit;
        changed
    }

    pub fn close_visible(&self, index: usize) -> bool {
        self.active == index
            || matches!(self.hovered, Some(TabButton::Select(i) | TabButton::Close(i)) if i == index)
    }

    pub fn marker(&self, index: usize) -> &'static str {
        if let Some(Some(state)) = self.agent_states.get(index) {
            return state.marker();
        }
        match self.activities.get(index).copied().unwrap_or_default() {
            TabActivity::Idle => "",
            TabActivity::Running => "●",
            TabActivity::Success => "✓",
            TabActivity::Failed => "!",
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        (0..self.titles.len())
            .filter(|index| {
                self.group_filter.as_ref().is_none_or(|group| {
                    self.groups.get(*index).and_then(Option::as_ref) == Some(group)
                })
            })
            .collect()
    }

    pub fn target(&self, action: TabAction) -> Option<usize> {
        let indices = self.visible_indices();
        let count = indices.len();
        if count == 0 {
            return None;
        }
        let active = indices.iter().position(|index| *index == self.active).unwrap_or(0);
        let position = match action {
            TabAction::Next => (active + 1) % count,
            TabAction::Previous => (active + count - 1) % count,
            TabAction::Index(index) => index,
            TabAction::Last => count - 1,
        };
        indices.get(position).copied()
    }

    pub fn group_title(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        if width < 5 {
            return "▾".into();
        }
        format!(" {} ▾", Self::shorten_label(&self.group_label, width - 4))
    }

    pub fn layout(&self, columns: usize) -> TabLayout {
        let indices = self.visible_indices();
        let has_groups = self.groups.iter().any(Option::is_some);
        let tab_start = if has_groups {
            (unicode_width::UnicodeWidthStr::width(self.group_label.as_str()) + 4)
                .clamp(8, 18)
                .min(columns.saturating_sub(12))
        } else {
            0
        };
        let available = columns.saturating_sub(
            tab_start + if self.agent_states.iter().any(Option::is_some) { 9 } else { 6 },
        );
        let count = indices.len().max(1);
        let width = (available / count).clamp(6, 30).min(available);
        let visible = if width == 0 { 0 } else { (available / width).min(indices.len()) };
        let active = indices.iter().position(|index| *index == self.active).unwrap_or(0);
        let offset = active.saturating_sub(visible.saturating_sub(1));
        let indices: Vec<_> = indices.into_iter().skip(offset).take(visible).collect();
        TabLayout {
            indices,
            tab_start,
            first: offset,
            visible,
            width,
            button_start: tab_start + visible * width,
        }
    }

    pub fn hit(&self, column: usize, columns: usize) -> Option<TabButton> {
        if column >= columns {
            return None;
        }
        let layout = self.layout(columns);
        if column < layout.tab_start {
            return Some(TabButton::Groups);
        }
        if column >= layout.button_start {
            return if column < layout.button_start + 3 {
                Some(TabButton::Create)
            } else if column < layout.button_start + 6 {
                Some(TabButton::Menu)
            } else if column < layout.button_start + 9
                && self.agent_states.iter().any(Option::is_some)
            {
                Some(TabButton::Agents)
            } else {
                None
            };
        }
        let column = column - layout.tab_start;
        let index = *layout.indices.get(column.checked_div(layout.width)?)?;
        if layout.width >= 5
            && (layout.width - 3..layout.width - 1).contains(&(column % layout.width))
        {
            Some(TabButton::Close(index))
        } else {
            Some(TabButton::Select(index))
        }
    }

    pub fn label(&self, index: usize, width: usize) -> String {
        let title = self.titles.get(index).map(String::as_str).unwrap_or_default();
        if let Some(agent) = self.agent_labels.get(index).filter(|value| !value.is_empty()) {
            if width < 14 {
                return Self::shorten_label(agent, width);
            }
            let name_width = (width / 3).max(4);
            return format!(
                "{} · {}",
                Self::shorten_label(title, name_width),
                Self::shorten_label(agent, width.saturating_sub(name_width + 3))
            );
        }
        Self::shorten_label(title, width)
    }

    fn shorten_label(title: &str, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let title: String = title
            .chars()
            .filter(|ch| !ch.is_control() && ch.width().is_some_and(|width| width > 0))
            .collect();
        let truncated = unicode_width::UnicodeWidthStr::width(title.as_str()) > width;
        let limit = width.saturating_sub(usize::from(truncated));
        let mut label = String::new();
        let mut used = 0;
        for ch in title.chars() {
            let cell_width = ch.width().unwrap_or_default();
            if used + cell_width > limit {
                break;
            }
            label.push(ch);
            used += cell_width;
        }
        if truncated {
            label.push('…');
        }
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_bar_reserves_one_row_without_clipping_the_bottom() {
        let mut size = crate::display::SizeInfo::new(800., 600., 10., 20., 4., 6., false);
        let original_lines = alacritty_terminal::grid::Dimensions::screen_lines(&size);
        size.reserve_top_lines(TabBar::ROWS);
        assert_eq!(size.padding_y(), 26.);
        assert_eq!(size.bottom_padding_y(), 6.);
        let lines = alacritty_terminal::grid::Dimensions::screen_lines(&size);
        assert_eq!(lines, original_lines - TabBar::ROWS);
        assert!(
            size.padding_y() + lines as f32 * size.cell_height()
                <= size.height() - size.bottom_padding_y()
        );
    }

    #[test]
    fn switching_preserves_order_and_state() {
        let mut tabs = Tabs::new(String::from("first"));
        tabs.push(String::from("second"));
        tabs.push(String::from("third"));
        assert!(tabs.select(2));
        tabs.active.push_str(" updated");
        assert!(tabs.select(0));
        assert!(tabs.select(1));
        assert_eq!(tabs.iter().map(String::as_str).collect::<Vec<_>>(), [
            "first",
            "second",
            "third updated"
        ]);
        assert!(!tabs.select(9));
        assert_eq!(tabs.active, "second");
    }

    #[test]
    fn moving_active_and_inactive_tabs_preserves_order_and_selection() {
        for active in 0..5 {
            for from in 0..5 {
                for to in 0..5 {
                    let mut tabs = Tabs::new(Box::new(0));
                    for value in 1..5 {
                        tabs.push(Box::new(value));
                    }
                    tabs.select(active);
                    let active_ptr = tabs.active.as_ref() as *const i32;
                    let mut expected = vec![0, 1, 2, 3, 4];
                    let moved = expected.remove(from);
                    expected.insert(to, moved);
                    assert_eq!(tabs.move_to(from, to), from != to);
                    assert_eq!(tabs.iter().map(|tab| **tab).collect::<Vec<_>>(), expected);
                    assert_eq!(*tabs.active, active as i32);
                    assert_eq!(tabs.active.as_ref() as *const i32, active_ptr);
                    assert_eq!(
                        tabs.active_index,
                        expected.iter().position(|&tab| tab == active as i32).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn moving_invalid_or_unchanged_tabs_leaves_order_intact() {
        let mut tabs = Tabs::new(0);
        tabs.push(1);
        tabs.push(2);
        tabs.select(1);
        for (from, to) in [(0, 0), (1, 1), (3, 0), (0, 3), (usize::MAX, 1), (1, usize::MAX)] {
            assert!(!tabs.move_to(from, to));
            assert_eq!(tabs.iter().copied().collect::<Vec<_>>(), [0, 1, 2]);
            assert_eq!(tabs.active_index, 1);
            assert_eq!(tabs.active, 1);
        }
        let mut single = Tabs::new(0);
        assert!(!single.move_to(0, 0));
        assert!(!single.move_to(0, 1));
        assert_eq!(single.active_index, 0);
        assert_eq!(single.active, 0);
    }

    #[test]
    fn extracting_active_and_inactive_tabs_retains_owned_resources() {
        struct Owned(std::rc::Rc<std::cell::Cell<usize>>, usize);
        impl Drop for Owned {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        for active in 0..4 {
            for removed in 0..4 {
                let dropped = std::rc::Rc::new(std::cell::Cell::new(0));
                let mut tabs = Tabs::new(Owned(dropped.clone(), 0));
                for id in 1..4 {
                    tabs.push(Owned(dropped.clone(), id));
                }
                tabs.select(active);
                let extracted = tabs.take(removed).unwrap();
                assert_eq!(extracted.1, removed);
                assert_eq!(dropped.get(), 0);
                assert_eq!(
                    tabs.iter().map(|tab| tab.1).collect::<Vec<_>>(),
                    (0..4).filter(|id| *id != removed).collect::<Vec<_>>()
                );
                if active != removed {
                    assert_eq!(tabs.active.1, active);
                }
                assert_eq!(tabs.get(tabs.active_index).unwrap().1, tabs.active.1);
                drop(extracted);
                assert_eq!(dropped.get(), 1);
                assert!(tabs.take(99).is_none());
                drop(tabs);
                assert_eq!(dropped.get(), 4);
            }
        }
        let mut single = Tabs::new(42);
        assert!(single.take(0).is_none());
        assert_eq!(single.active, 42);
    }

    #[test]
    fn closing_active_and_inactive_tabs_preserves_selection() {
        let mut tabs = Tabs::new(0);
        for value in 1..4 {
            tabs.push(value);
        }
        tabs.select(2);
        assert!(tabs.remove(0));
        assert_eq!(tabs.active, 2);
        assert_eq!(tabs.active_index, 1);
        assert!(tabs.remove(1));
        assert_eq!(tabs.active, 3);
        assert_eq!(tabs.iter().copied().collect::<Vec<_>>(), [1, 3]);
        assert!(tabs.remove(1));
        assert_eq!(tabs.active, 1);
        assert!(!tabs.remove(0));
    }

    #[test]
    fn cycling_wraps_and_visits_each_session() {
        let mut tabs = Tabs::new(0);
        tabs.push(1);
        tabs.push(2);
        assert_eq!(tabs.target(TabAction::Previous), 2);
        tabs.select(2);
        assert_eq!(tabs.target(TabAction::Next), 0);
        assert_eq!(tabs.target(TabAction::Last), 2);
        for tab in tabs.iter_mut() {
            *tab += 10;
        }
        assert_eq!(tabs.iter().copied().collect::<Vec<_>>(), [10, 11, 12]);
    }

    #[test]
    fn group_navigation_uses_member_positions_and_original_indices() {
        let bar = TabBar {
            titles: vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            groups: vec![
                Some("work".into()),
                None,
                Some("work".into()),
                Some("home".into()),
                Some("work".into()),
            ],
            group_filter: Some("work".into()),
            active: 2,
            ..Default::default()
        };
        assert_eq!(bar.visible_indices(), vec![0, 2, 4]);
        assert_eq!(bar.target(TabAction::Next), Some(4));
        assert_eq!(bar.target(TabAction::Previous), Some(0));
        assert_eq!(bar.target(TabAction::Index(1)), Some(2));
        assert_eq!(bar.target(TabAction::Index(3)), None);
        assert_eq!(bar.target(TabAction::Last), Some(4));
        let bar = TabBar { active: 4, ..bar };
        assert_eq!(bar.target(TabAction::Next), Some(0));
        let bar = TabBar { active: 0, ..bar };
        assert_eq!(bar.target(TabAction::Previous), Some(4));
        let bar = TabBar { group_filter: Some("missing".into()), ..bar };
        assert!(bar.visible_indices().is_empty());
        assert_eq!(bar.target(TabAction::Next), None);
        assert_eq!(bar.layout(80).visible, 0);
    }

    #[test]
    fn group_selector_and_filtered_tabs_share_mouse_geometry() {
        let bar = TabBar {
            titles: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            groups: vec![None, Some("work".into()), None, Some("work".into())],
            group_filter: Some("work".into()),
            group_label: "work".into(),
            active: 3,
            ..Default::default()
        };
        let layout = bar.layout(80);
        assert_eq!(layout.indices, vec![1, 3]);
        for column in 0..layout.tab_start {
            assert_eq!(bar.hit(column, 80), Some(TabButton::Groups));
        }
        for (slot, index) in layout.indices.iter().copied().enumerate() {
            let start = layout.tab_start + slot * layout.width;
            assert_eq!(bar.hit(start, 80), Some(TabButton::Select(index)));
            assert_eq!(bar.hit(start + layout.width - 3, 80), Some(TabButton::Close(index)));
        }
        assert_eq!(bar.hit(layout.button_start, 80), Some(TabButton::Create));
        assert_eq!(bar.hit(layout.button_start + 3, 80), Some(TabButton::Menu));
    }

    #[test]
    fn grouped_overflow_keeps_the_active_member_visible() {
        let bar = TabBar {
            titles: vec!["shell".into(); 30],
            groups: (0..30).map(|index| (index % 2 == 1).then(|| "work".into())).collect(),
            group_filter: Some("work".into()),
            group_label: "work".into(),
            active: 29,
            ..Default::default()
        };
        let layout = bar.layout(40);
        assert!(layout.indices.contains(&29));
        assert!(layout.indices.iter().all(|index| index % 2 == 1));
        assert_eq!(
            bar.hit(layout.tab_start, 40),
            layout.indices.first().copied().map(TabButton::Select)
        );
        assert!(layout.button_start + 6 <= 40);
        for columns in 0..30 {
            let layout = bar.layout(columns);
            assert!(layout.tab_start <= columns);
            assert!(layout.indices.len() == layout.visible);
            for column in 0..columns {
                if let Some(TabButton::Select(index) | TabButton::Close(index)) =
                    bar.hit(column, columns)
                {
                    assert!(bar.visible_indices().contains(&index));
                }
            }
            assert_eq!(bar.hit(columns, columns), None);
        }
    }

    #[test]
    fn group_labels_fit_unicode_width_and_strip_controls() {
        let bar = TabBar { group_label: "日本語\n\x1blong".into(), ..Default::default() };
        for width in 0..25 {
            let label = bar.group_title(width);
            assert!(unicode_width::UnicodeWidthStr::width(label.as_str()) <= width);
            assert!(!label.chars().any(char::is_control));
        }
        let ungrouped = TabBar { titles: vec!["one".into()], ..Default::default() };
        assert_eq!(ungrouped.layout(80).tab_start, 0);
        assert_eq!(ungrouped.hit(0, 80), Some(TabButton::Select(0)));
    }

    #[test]
    fn hovering_reveals_close_without_changing_selection() {
        let mut bar =
            TabBar { titles: vec!["first".into(), "second".into()], ..Default::default() };
        assert!(bar.close_visible(0));
        assert!(!bar.close_visible(1));
        assert!(bar.hover(Some(TabButton::Select(1))));
        assert!(!bar.hover(Some(TabButton::Select(1))));
        assert!(bar.close_visible(1));
        assert_eq!(bar.active, 0);
        let layout = bar.layout(80);
        for column in layout.width * 2 - 3..layout.width * 2 - 1 {
            assert_eq!(bar.hit(column, 80), Some(TabButton::Close(1)));
        }
        assert_eq!(bar.hit(layout.width * 2 - 1, 80), Some(TabButton::Select(1)));
        assert!(bar.hover(None));
        assert!(!bar.close_visible(1));
        assert!(bar.close_visible(0));
    }

    #[test]
    fn narrow_bar_keeps_active_tab_visible_and_routes_buttons() {
        let bar = TabBar { titles: vec!["shell".into(); 12], active: 11, ..Default::default() };
        let layout = bar.layout(40);
        assert!(layout.first <= bar.active);
        assert!(layout.first + layout.visible > bar.active);
        assert_eq!(bar.hit(0, 40), Some(TabButton::Select(layout.first)));
        assert_eq!(bar.hit(layout.width - 2, 40), Some(TabButton::Close(layout.first)));
        assert_eq!(bar.hit(layout.button_start + 1, 40), Some(TabButton::Create));
        assert_eq!(bar.hit(40, 40), None);
        assert_eq!(bar.layout(2).visible, 0);
    }

    #[test]
    fn agent_attention_stays_discoverable_outside_the_selected_group() {
        let bar = TabBar {
            titles: vec!["Editor".into(), "Worker".into()],
            groups: vec![Some("code".into()), Some("agents".into())],
            group_filter: Some("code".into()),
            active: 0,
            agent_states: vec![None, Some(crate::agent::AgentState::Permission)],
            agent_labels: vec![String::new(), "Permission · Claude".into()],
            agent_unread: 1,
            ..Default::default()
        };
        assert_eq!(bar.visible_indices(), vec![0]);
        assert_eq!(bar.marker(1), "!");
        for columns in 0..120 {
            let layout = bar.layout(columns);
            for column in layout.button_start + 6..(layout.button_start + 9).min(columns) {
                assert_eq!(bar.hit(column, columns), Some(TabButton::Agents));
            }
        }
        assert_eq!(bar.titles[1], "Worker");
        assert!(bar.label(1, 23).contains("Permission"));
    }

    #[test]
    fn activity_markers_leave_custom_titles_unchanged() {
        let mut bar = TabBar { titles: vec!["shell".into()], ..Default::default() };
        for (activity, marker) in [
            (TabActivity::Idle, ""),
            (TabActivity::Running, "●"),
            (TabActivity::Success, "✓"),
            (TabActivity::Failed, "!"),
        ] {
            bar.activities = vec![activity];
            assert_eq!(bar.marker(0), marker);
            assert_eq!(bar.label(0, 12), "shell");
        }
    }

    #[test]
    fn activity_labels_fit_narrow_and_unicode_titles() {
        let bar = TabBar {
            titles: vec!["日本語".into()],
            activities: vec![TabActivity::Running],
            ..Default::default()
        };
        for width in 0..20 {
            let label = bar.label(0, width);
            assert!(unicode_width::UnicodeWidthStr::width(label.as_str()) <= width);
            if (1..6).contains(&width) {
                assert!(label.ends_with('…'));
            }
        }
        assert_eq!(bar.label(0, 6), "日本語");
    }

    #[test]
    fn labels_fit_unicode_cell_width_and_strip_controls() {
        let bar =
            TabBar { titles: vec!["\u{301}日本語\n\x1b title".into()], ..Default::default() };
        let label = bar.label(0, 12);
        assert!(unicode_width::UnicodeWidthStr::width(label.as_str()) <= 12);
        assert!(!label.chars().any(char::is_control));
        assert!(label.chars().all(|ch| ch.width().is_some_and(|width| width > 0)));
    }
}
