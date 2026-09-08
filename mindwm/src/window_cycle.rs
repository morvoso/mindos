//! Recent-window switching with a stable order while Alt/Super is held.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleModifier { Alt, Super }

#[derive(Debug)]
struct Selection {
    order: Vec<u64>,
    index: usize,
    origin: Option<u64>,
    modifier: CycleModifier,
}

#[derive(Debug, Default)]
pub struct WindowCycle {
    recent: Vec<u64>,
    selection: Option<Selection>,
}

impl WindowCycle {
    pub fn active(&self) -> bool { self.selection.is_some() }

    pub fn focus(&mut self, id: u64) {
        if self.selection.as_ref().is_some_and(|s| s.order[s.index] == id) {
            return; // Previewing does not reorder the next Alt+Tab session.
        }
        self.selection = None;
        self.recent.retain(|old| *old != id);
        self.recent.insert(0, id);
    }

    pub fn retain(&mut self, available: &[u64]) {
        self.recent.retain(|id| available.contains(id));
    }

    pub fn finish(&mut self) {
        if let Some(selection) = self.selection.take() {
            self.focus(selection.order[selection.index]);
        }
    }

    pub fn release(&mut self, alt: bool, logo: bool) {
        if self.selection.as_ref().is_some_and(|s| match s.modifier {
            CycleModifier::Alt => !alt, CycleModifier::Super => !logo,
        }) { self.finish(); }
    }

    pub fn cancel(&mut self) -> Option<u64> {
        self.selection.take().and_then(|s| s.origin)
    }

    pub fn step(&mut self, available: &[u64], focused: Option<u64>, reverse: bool,
                modifier: CycleModifier) -> Option<u64> {
        self.retain(available);
        if available.is_empty() { self.selection = None; return None; }
        if self.selection.as_ref().is_some_and(|s| s.modifier != modifier) { self.finish(); }
        if self.selection.is_none() {
            if let Some(id) = focused.filter(|id| available.contains(id)) { self.focus(id); }
            let mut order = self.recent.clone();
            for id in available { if !order.contains(id) { order.push(*id); } }
            let index = focused.and_then(|id| order.iter().position(|item| *item == id))
                .unwrap_or(if reverse { 0 } else { order.len() - 1 });
            self.selection = Some(Selection { order, index, origin: focused, modifier });
        }
        let selection = self.selection.as_mut().unwrap();
        let n = selection.order.len();
        for _ in 0..n {
            selection.index = if reverse { (selection.index + n - 1) % n } else { (selection.index + 1) % n };
            let id = selection.order[selection.index];
            if available.contains(&id) { return Some(id); }
        }
        self.selection = None;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_keep_order_and_releasing_commits_recent_window() {
        let mut cycle = WindowCycle::default();
        for id in [1, 2, 3] { cycle.focus(id); }
        assert_eq!(cycle.step(&[1, 2, 3], Some(3), false, CycleModifier::Alt), Some(2));
        cycle.focus(2);
        assert_eq!(cycle.step(&[1, 2, 3], Some(2), false, CycleModifier::Alt), Some(1));
        cycle.focus(1);
        cycle.release(true, false);
        assert!(cycle.active());
        cycle.release(false, false);
        assert_eq!(cycle.step(&[1, 2, 3], Some(1), false, CycleModifier::Alt), Some(3));
    }
    #[test]
    fn reverse_cancel_and_closed_windows() {
        let mut cycle = WindowCycle::default();
        for id in [1, 2, 3] { cycle.focus(id); }
        assert_eq!(cycle.step(&[1, 2, 3], Some(3), true, CycleModifier::Super), Some(1));
        assert_eq!(cycle.step(&[2, 3], Some(1), true, CycleModifier::Super), Some(2));
        assert_eq!(cycle.cancel(), Some(3));
        assert!(!cycle.active());
        assert_eq!(cycle.step(&[], None, false, CycleModifier::Alt), None);
    }
    #[test]
    fn unrelated_focus_ends_switching_and_no_focus_starts_at_first() {
        let mut cycle = WindowCycle::default();
        assert_eq!(cycle.step(&[3, 2, 1], None, false, CycleModifier::Alt), Some(3));
        cycle.focus(2);
        assert!(!cycle.active());
        assert_eq!(cycle.step(&[3, 2, 1], Some(2), false, CycleModifier::Alt), Some(3));
    }
}
