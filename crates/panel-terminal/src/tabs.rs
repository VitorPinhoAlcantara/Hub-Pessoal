#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Empty,
    Activated(u64),
    Unchanged,
}

#[derive(Debug, Default, Clone)]
pub struct Tabs {
    ids: Vec<u64>,
    active: Option<u64>,
}

impl Tabs {
    pub fn add(&mut self, id: u64) {
        self.ids.push(id);
        self.active = Some(id);
    }

    pub fn select(&mut self, id: u64) -> bool {
        if self.ids.contains(&id) {
            self.active = Some(id);
            true
        } else {
            false
        }
    }

    pub fn active(&self) -> Option<u64> {
        self.active
    }

    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn remove(&mut self, id: u64) -> Removal {
        let Some(position) = self.ids.iter().position(|candidate| *candidate == id) else {
            return Removal::Unchanged;
        };
        self.ids.remove(position);
        if self.ids.is_empty() {
            self.active = None;
            return Removal::Empty;
        }
        if self.active != Some(id) {
            return Removal::Unchanged;
        }
        let next = self.ids[position.min(self.ids.len() - 1)];
        self.active = Some(next);
        Removal::Activated(next)
    }
}
