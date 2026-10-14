use std::any::{Any, TypeId, type_name};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;

use super::entity::Entities;
use super::{Entity, SparseSet};

trait ErasedStorage {
    fn remove_entity(&mut self, e: Entity);
    fn as_any(&self) -> &dyn Any;
}

impl<T: 'static> ErasedStorage for RefCell<SparseSet<T>> {
    fn remove_entity(&mut self, e: Entity) {
        self.get_mut().remove(e);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Сущности и хранилища компонентов, по одному `RefCell` на тип.
#[derive(Default)]
pub struct World {
    entities: Entities,
    storages: HashMap<TypeId, Box<dyn ErasedStorage>>,
}

impl World {
    /// Повторная регистрация — no-op.
    pub fn register<T: 'static>(&mut self) {
        self.storages
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(RefCell::new(SparseSet::<T>::default())));
    }

    pub fn spawn(&mut self) -> Entity {
        self.entities.spawn()
    }

    /// Паника на мёртвую `e`: вставка в её слот дала бы призрачную сущность в обходе.
    pub fn insert<T: 'static>(&mut self, e: Entity, value: T) {
        assert!(self.is_alive(e), "insert в мёртвую {e:?}");
        self.storage_mut::<T>().insert(e, value);
    }

    /// No-op на мёртвую `e`.
    pub fn despawn(&mut self, e: Entity) {
        if self.entities.despawn(e) {
            for storage in self.storages.values_mut() {
                storage.remove_entity(e);
            }
        }
    }

    pub fn is_alive(&self, e: Entity) -> bool {
        self.entities.is_alive(e)
    }

    pub fn storage<T: 'static>(&self) -> Ref<'_, SparseSet<T>> {
        self.cell::<T>()
            .try_borrow()
            .unwrap_or_else(|_| panic!("хранилище {} занято на запись", type_name::<T>()))
    }

    pub fn storage_mut<T: 'static>(&self) -> RefMut<'_, SparseSet<T>> {
        self.cell::<T>()
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("хранилище {} уже заимствовано", type_name::<T>()))
    }

    fn cell<T: 'static>(&self) -> &RefCell<SparseSet<T>> {
        self.storages
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("компонент {} не зарегистрирован", type_name::<T>()))
            .as_any()
            .downcast_ref()
            .expect("хранилище другого типа под TypeId")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Pos(i32);

    #[derive(Debug, PartialEq)]
    struct Vel(i32);

    fn world() -> World {
        let mut world = World::default();
        world.register::<Pos>();
        world.register::<Vel>();
        world
    }

    #[test]
    fn stale_entity_gets_none() {
        let mut world = world();
        let old = world.spawn();
        world.insert(old, Pos(1));
        world.despawn(old);
        let new = world.spawn();
        world.insert(new, Pos(2));
        assert_eq!(new.index, old.index);
        assert_eq!(world.storage::<Pos>().get(old), None);
        assert_eq!(world.storage::<Pos>().get(new), Some(&Pos(2)));
    }

    #[test]
    fn despawn_removes_from_all_storages() {
        let mut world = world();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(a, Pos(1));
        world.insert(a, Vel(1));
        world.insert(b, Pos(2));
        world.despawn(a);
        assert!(!world.is_alive(a));
        assert_eq!(world.storage::<Vel>().len(), 0);
        let pos: Vec<_> = world.storage::<Pos>().iter().map(|(e, _)| e).collect();
        assert_eq!(pos, [b]);
        world.despawn(a);
        assert_eq!(world.storage::<Pos>().len(), 1);
    }

    #[test]
    #[should_panic(expected = "insert в мёртвую")]
    fn insert_into_dead_panics() {
        let mut world = world();
        let e = world.spawn();
        world.despawn(e);
        world.insert(e, Pos(1));
    }

    #[test]
    #[should_panic(expected = "Pos не зарегистрирован")]
    fn unregistered_storage_panics_with_name() {
        let world = World::default();
        let _ = world.storage::<Pos>();
    }

    #[test]
    #[should_panic(expected = "Vel не зарегистрирован")]
    fn insert_unregistered_panics_with_name() {
        let mut world = World::default();
        world.register::<Pos>();
        let e = world.spawn();
        world.insert(e, Vel(1));
    }

    #[test]
    fn read_one_storage_while_writing_another() {
        let mut world = world();
        let e = world.spawn();
        world.insert(e, Pos(1));
        world.insert(e, Vel(3));
        let vel = world.storage::<Vel>();
        let mut pos = world.storage_mut::<Pos>();
        for (e, p) in pos.iter_mut() {
            p.0 += vel.get(e).unwrap().0;
        }
        assert_eq!(pos.get(e), Some(&Pos(4)));
    }

    #[test]
    #[should_panic(expected = "Pos уже заимствовано")]
    fn two_mut_borrows_of_one_storage_panic() {
        let world = world();
        let _a = world.storage_mut::<Pos>();
        let _b = world.storage_mut::<Pos>();
    }
}
