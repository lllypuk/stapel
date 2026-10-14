use super::Entity;

const NONE: u32 = u32::MAX;

fn to_u32(i: usize) -> u32 {
    u32::try_from(i).expect("больше u32::MAX компонентов одного типа")
}

/// Плотное хранилище компонента `T`, индексируемое слотом сущности.
pub struct SparseSet<T> {
    sparse: Vec<u32>,
    dense: Vec<T>,
    entities: Vec<Entity>,
}

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            entities: Vec::new(),
        }
    }
}

impl<T> SparseSet<T> {
    fn position(&self, e: Entity) -> Option<usize> {
        let i = *self.sparse.get(e.index as usize)?;
        (i != NONE && self.entities[i as usize] == e).then_some(i as usize)
    }

    /// Компонент `e`; `None`, если его нет.
    pub fn get(&self, e: Entity) -> Option<&T> {
        self.position(e).map(|i| &self.dense[i])
    }

    /// Компонент `e` на запись.
    pub fn get_mut(&mut self, e: Entity) -> Option<&mut T> {
        self.position(e).map(|i| &mut self.dense[i])
    }

    /// Пары сущность–компонент; порядок меняется при удалении.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.entities.iter().copied().zip(&self.dense)
    }

    /// Пары сущность–компонент на запись.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.entities.iter().copied().zip(&mut self.dense)
    }

    /// Число компонентов.
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// Нет ни одного компонента.
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// Не проверяет живость `e`: вызывается только из `World`.
    pub(crate) fn insert(&mut self, e: Entity, value: T) {
        let slot = e.index as usize;
        if slot >= self.sparse.len() {
            self.sparse.resize(slot + 1, NONE);
        }
        match self.sparse[slot] {
            NONE => {
                self.sparse[slot] = to_u32(self.dense.len());
                self.dense.push(value);
                self.entities.push(e);
            }
            i => {
                debug_assert_eq!(self.entities[i as usize], e, "чужая сущность в слоте");
                self.dense[i as usize] = value;
            }
        }
    }

    pub(crate) fn remove(&mut self, e: Entity) -> Option<T> {
        let i = self.position(e)?;
        let value = self.dense.swap_remove(i);
        self.entities.swap_remove(i);
        if let Some(moved) = self.entities.get(i) {
            self.sparse[moved.index as usize] = to_u32(i);
        }
        self.sparse[e.index as usize] = NONE;
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(index: u32, generation: u32) -> Entity {
        Entity { index, generation }
    }

    #[test]
    fn stale_entity_misses() {
        let mut set = SparseSet::default();
        set.insert(entity(0, 1), 'a');
        assert_eq!(set.get(entity(0, 0)), None);
        assert_eq!(set.get(entity(0, 1)), Some(&'a'));
        assert_eq!(set.remove(entity(0, 0)), None);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn swap_remove_repoints_moved() {
        let mut set = SparseSet::default();
        let (a, b, c) = (entity(0, 0), entity(5, 0), entity(2, 0));
        set.insert(a, 'a');
        set.insert(b, 'b');
        set.insert(c, 'c');
        assert_eq!(set.remove(a), Some('a'));
        assert_eq!(set.get(c), Some(&'c'));
        assert_eq!(set.get(b), Some(&'b'));
        *set.get_mut(c).unwrap() = 'C';
        let items: Vec<_> = set.iter().collect();
        assert_eq!(items, [(c, &'C'), (b, &'b')]);
    }

    #[test]
    fn insert_twice_replaces() {
        let mut set = SparseSet::default();
        let a = entity(3, 0);
        set.insert(a, 'a');
        set.insert(a, 'A');
        assert_eq!(set.get(a), Some(&'A'));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn remove_last() {
        let mut set = SparseSet::default();
        let (a, b) = (entity(0, 0), entity(1, 0));
        set.insert(a, 'a');
        set.insert(b, 'b');
        assert_eq!(set.remove(b), Some('b'));
        assert_eq!(set.get(b), None);
        assert_eq!(set.get(a), Some(&'a'));
        assert_eq!(set.remove(a), Some('a'));
        assert!(set.is_empty());
    }

    #[test]
    fn not_empty_after_insert() {
        let mut set = SparseSet::default();
        assert!(set.is_empty());
        set.insert(entity(0, 0), 'a');
        assert!(!set.is_empty());
    }
}
