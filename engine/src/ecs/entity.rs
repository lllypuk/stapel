/// Слот и поколение: устаревшая копия не совпадает с новой сущностью в том же слоте.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Entity {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

#[derive(Clone, Copy)]
struct Slot {
    generation: u32,
    alive: bool,
}

/// Аллокатор сущностей со списком свободных слотов.
#[derive(Default)]
pub(crate) struct Entities {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl Entities {
    pub(crate) fn spawn(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.alive = true;
            return Entity {
                index,
                generation: slot.generation,
            };
        }
        let index = u32::try_from(self.slots.len()).expect("слоты сущностей исчерпаны");
        self.slots.push(Slot {
            generation: 0,
            alive: true,
        });
        Entity {
            index,
            generation: 0,
        }
    }

    /// `false`, если `e` уже мертва.
    pub(crate) fn despawn(&mut self, e: Entity) -> bool {
        if !self.is_alive(e) {
            return false;
        }
        let slot = &mut self.slots[e.index as usize];
        slot.generation = slot.generation.wrapping_add(1);
        slot.alive = false;
        self.free.push(e.index);
        true
    }

    pub(crate) fn is_alive(&self, e: Entity) -> bool {
        self.slots
            .get(e.index as usize)
            .is_some_and(|s| s.alive && s.generation == e.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn despawned_index_reused_with_next_generation() {
        let mut entities = Entities::default();
        let a = entities.spawn();
        let b = entities.spawn();
        assert!(entities.despawn(a));
        let c = entities.spawn();
        assert_eq!((c.index, c.generation), (a.index, a.generation + 1));
        assert!(!entities.is_alive(a));
        assert!(entities.is_alive(b) && entities.is_alive(c));
    }

    #[test]
    fn despawn_dead_is_noop() {
        let mut entities = Entities::default();
        let a = entities.spawn();
        assert!(entities.despawn(a));
        assert!(!entities.despawn(a));
        assert_eq!(entities.spawn().generation, 1);
    }
}
