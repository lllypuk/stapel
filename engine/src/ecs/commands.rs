use std::collections::HashSet;

use super::{Entity, World};

type SpawnFn = Box<dyn FnOnce(&mut World)>;

/// Структурные изменения, отложенные до конца обхода.
#[derive(Default)]
pub struct Commands {
    despawns: HashSet<Entity>,
    spawns: Vec<SpawnFn>,
}

impl Commands {
    /// Сущности создаются только в `apply`; одна closure может создать сколько угодно.
    pub fn spawn(&mut self, f: impl FnOnce(&mut World) + 'static) {
        self.spawns.push(Box::new(f));
    }

    pub fn despawn(&mut self, e: Entity) {
        self.despawns.insert(e);
    }

    pub fn is_pending(&self, e: Entity) -> bool {
        self.despawns.contains(&e)
    }

    /// Сначала despawn, потом spawn в порядке постановки.
    pub fn apply(&mut self, world: &mut World) {
        for e in self.despawns.drain() {
            world.despawn(e);
        }
        for f in self.spawns.drain(..) {
            f(world);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Tag(u32);

    fn world() -> World {
        let mut world = World::default();
        world.register::<Tag>();
        world
    }

    fn tags(world: &World) -> Vec<u32> {
        world.storage::<Tag>().iter().map(|(_, t)| t.0).collect()
    }

    #[test]
    fn despawn_runs_before_spawn() {
        let mut world = world();
        let old = world.spawn();
        let mut commands = Commands::default();
        commands.spawn(move |w| {
            assert!(!w.is_alive(old));
            let e = w.spawn();
            assert_eq!(e.index, old.index);
            w.insert(e, Tag(1));
        });
        commands.despawn(old);
        commands.apply(&mut world);
        assert_eq!(tags(&world), [1]);
    }

    #[test]
    fn spawns_keep_order() {
        let mut world = world();
        let mut commands = Commands::default();
        for i in 0..3 {
            commands.spawn(move |w| {
                let e = w.spawn();
                w.insert(e, Tag(i));
            });
        }
        commands.apply(&mut world);
        assert_eq!(tags(&world), [0, 1, 2]);
    }

    #[test]
    fn double_despawn_is_noop() {
        let mut world = world();
        let a = world.spawn();
        let b = world.spawn();
        world.insert(b, Tag(2));
        let mut commands = Commands::default();
        commands.despawn(a);
        commands.despawn(a);
        commands.apply(&mut world);
        assert!(!world.is_alive(a));
        assert!(world.is_alive(b));
        assert_eq!(world.spawn().index, a.index);
        assert_ne!(world.spawn().index, a.index);
    }

    #[test]
    fn pending_until_apply() {
        let mut world = world();
        let a = world.spawn();
        let b = world.spawn();
        let mut commands = Commands::default();
        commands.despawn(a);
        assert!(commands.is_pending(a));
        assert!(!commands.is_pending(b));
        commands.apply(&mut world);
        assert!(!commands.is_pending(a));
    }

    #[test]
    fn despawn_of_dead_is_noop() {
        let mut world = world();
        let dead = world.spawn();
        world.despawn(dead);
        let reused = world.spawn();
        world.insert(reused, Tag(7));
        let mut commands = Commands::default();
        commands.despawn(dead);
        commands.apply(&mut world);
        assert!(world.is_alive(reused));
        assert_eq!(tags(&world), [7]);
    }

    #[test]
    fn one_closure_spawns_many() {
        let mut world = world();
        let mut commands = Commands::default();
        commands.spawn(|w| {
            for i in 0..100 {
                let e = w.spawn();
                w.insert(e, Tag(i));
            }
        });
        commands.apply(&mut world);
        assert_eq!(world.storage::<Tag>().len(), 100);
        commands.apply(&mut world);
        assert_eq!(world.storage::<Tag>().len(), 100);
    }
}
