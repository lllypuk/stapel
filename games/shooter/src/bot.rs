//! Бот сквозных тестов: проходит уровень через `Input`, как игрок.

use engine::{ElementState, IVec2, Input, KeyCode, MouseButton, Vec2};

use crate::Level;
use crate::enemy::Enemy;
use crate::obstacle::Obstacles;
use crate::player::SENSITIVITY;
use crate::weapon::RANGE;

/// Расстояние до центра клетки пути, с которого она пройдена; больше шага игрока за тик.
const ARRIVE: f32 = 0.05;

/// Ввод бота на тик.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Action {
    pub(crate) yaw: f32,
    pub(crate) walk: bool,
    pub(crate) fire: bool,
    /// Оружие, на которое переключиться.
    pub(crate) weapon: Option<usize>,
}

/// Бьёт видимого врага, иначе идёт по BFS к ближнему живому врагу, ключу, выходу.
#[derive(Debug, Default)]
pub(crate) struct Bot {
    goal: Option<IVec2>,
    /// Клетки пути к `goal` от последней.
    path: Vec<IVec2>,
}

impl Bot {
    pub(crate) fn act(&mut self, level: &Level, exit: IVec2) -> Action {
        let pos = level.player.pos;
        let arsenal = &level.arsenal;
        let weapon = (arsenal.weapon().ammo == 0)
            .then(|| arsenal.weapons.iter().position(|w| w.ammo > 0))
            .flatten();
        if let Some(enemy) = visible(level) {
            return Action {
                yaw: yaw(enemy - pos),
                walk: false,
                fire: true,
                weapon,
            };
        }
        // С ключом путь идёт сквозь двери: игрок откроет их касанием.
        let mut obstacles = level.obstacles.clone();
        if level.key {
            for door in &level.doors {
                obstacles.open(door.cell);
            }
        }
        let cell = pos.floor().as_ivec2();
        let goal = goal(level, &obstacles, cell).unwrap_or(exit);
        if self.goal != Some(goal) || self.path.is_empty() {
            self.goal = Some(goal);
            self.path = obstacles.path(cell, goal).unwrap_or_default();
            self.path.reverse();
        }
        if let Some(&next) = self.path.last()
            && pos.distance(centre(next)) <= ARRIVE
        {
            self.path.pop();
        }
        let to = self.path.last().map_or(pos, |&c| centre(c));
        Action {
            yaw: if to == pos {
                level.player.yaw
            } else {
                yaw(to - pos)
            },
            walk: to != pos,
            fire: false,
            weapon,
        }
    }
}

/// Ближний живой враг на линии выстрела.
fn visible(level: &Level) -> Option<Vec2> {
    let pos = level.player.pos;
    level
        .enemies
        .iter()
        .filter_map(|e| Some(e.target()?.pos))
        .filter(|&e| pos.distance(e) <= RANGE && level.obstacles.segment(pos, e).is_none())
        .min_by(|a, b| pos.distance(*a).total_cmp(&pos.distance(*b)))
}

/// Ближний по пути живой враг, иначе ключ, если его ещё нет в руках.
fn goal(level: &Level, obstacles: &Obstacles, cell: IVec2) -> Option<IVec2> {
    let enemies = level.enemies.iter().filter_map(Enemy::target);
    let keys = level.keys.iter().filter(|_| !level.key);
    nearest(obstacles, cell, enemies.map(|t| t.pos.floor().as_ivec2()))
        .or_else(|| nearest(obstacles, cell, keys.copied()))
}

/// Клетка из `cells` с кратчайшим путём от `cell`.
fn nearest(
    obstacles: &Obstacles,
    cell: IVec2,
    cells: impl Iterator<Item = IVec2>,
) -> Option<IVec2> {
    cells
        .filter_map(|to| Some((obstacles.path(cell, to)?.len(), to)))
        .min_by_key(|&(len, _)| len)
        .map(|(_, to)| to)
}

fn centre(cell: IVec2) -> Vec2 {
    cell.as_vec2() + 0.5
}

/// `yaw` игрока, смотрящего вдоль `d`: обратное `player::forward`.
fn yaw(d: Vec2) -> f32 {
    (-d.x).atan2(-d.y)
}

/// Действие — в события ввода для игрока с углом `yaw`.
pub(crate) fn press(action: Action, yaw: f32, input: &mut Input) {
    let state = |on: bool| {
        if on {
            ElementState::Pressed
        } else {
            ElementState::Released
        }
    };
    input.on_mouse_motion(Vec2::new((yaw - action.yaw) / SENSITIVITY, 0.0));
    input.on_key(KeyCode::KeyW, state(action.walk), false);
    input.on_button(MouseButton::Left, state(action.fire));
    for (i, key) in [KeyCode::Digit1, KeyCode::Digit2].into_iter().enumerate() {
        input.on_key(key, state(action.weapon == Some(i)), false);
    }
}

#[cfg(test)]
mod tests {
    use engine::{Context, ivec2, vec2};

    use super::*;
    use crate::Shooter;
    use crate::map::{Dir, Map};
    use crate::player::{Player, forward};

    fn level(text: &str) -> Level {
        Shooter::new(Map::parse(text).unwrap(), &mut Context::default().assets).level
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn yaw_inverts_forward() {
        for d in [
            vec2(1.0, 0.0),
            vec2(0.0, -1.0),
            vec2(-0.6, 0.8),
            vec2(0.0, 1.0),
        ] {
            assert!(forward(yaw(d)).distance(d) < 1e-6, "{d}");
        }
    }

    #[test]
    fn fires_at_nearest_visible_enemy_standing() {
        let mut l = level("#########\n#P..f.sX#\n#.......#\n#########\n");
        l.player.pos = vec2(4.0, 2.5);
        let a = Bot::default().act(&l, ivec2(7, 1));
        assert!(a.fire && !a.walk);
        assert!(near(a.yaw, yaw(vec2(0.5, -1.0))), "{}", a.yaw);

        l.enemies[0].hurt(999);
        let a = Bot::default().act(&l, ivec2(7, 1));
        assert!(
            near(a.yaw, yaw(vec2(2.5, -1.0))),
            "мёртвый не цель: {}",
            a.yaw
        );
    }

    #[test]
    fn walks_around_wall_to_hidden_enemy() {
        let l = level("#######\n#P.R.f#\n#....X#\n#######\n");
        let a = Bot::default().act(&l, ivec2(5, 2));
        assert!(!a.fire && a.walk);
        assert!(
            near(a.yaw, Dir::E.yaw()) || near(a.yaw, Dir::S.yaw()),
            "{}",
            a.yaw
        );
    }

    #[test]
    fn goes_for_key_then_through_closed_door_to_exit() {
        let mut l = level("#########\n#k..P.DX#\n#########\n");
        let mut bot = Bot::default();
        assert!(near(bot.act(&l, ivec2(7, 1)).yaw, Dir::W.yaw()));
        l.keys.clear();
        l.key = true;
        let a = bot.act(&l, ivec2(7, 1));
        assert!(a.walk && near(a.yaw, Dir::E.yaw()), "{a:?}");
    }

    #[test]
    fn follows_path_cell_by_cell() {
        let l = level("#####\n#P..#\n###.#\n#X..#\n#####\n");
        let exit = ivec2(1, 3);
        let mut bot = Bot::default();
        let mut player = Player::new(l.player.pos, l.player.yaw);
        let mut input = Input::default();
        for _ in 0..600 {
            let mut now = l.clone();
            now.player = player;
            press(bot.act(&now, exit), player.yaw, &mut input);
            player.update(&input, 1.0 / 60.0, &l.obstacles);
            input.end_tick();
            if player.pos.floor().as_ivec2() == exit {
                return;
            }
        }
        panic!("застрял в {:?}", player.pos);
    }

    #[test]
    fn stands_when_goal_unreachable() {
        let l = level("######\n#P#.X#\n######\n");
        let a = Bot::default().act(&l, ivec2(4, 1));
        assert!(!a.walk && !a.fire);
        assert_eq!(a.yaw, l.player.yaw);
    }

    #[test]
    fn switches_to_loaded_weapon_only_when_empty() {
        let mut l = level("#####\n#P.X#\n#####\n");
        let exit = ivec2(3, 1);
        assert_eq!(Bot::default().act(&l, exit).weapon, None);
        l.arsenal.weapons[0].ammo = 0;
        assert_eq!(Bot::default().act(&l, exit).weapon, Some(1));
        l.arsenal.current = 1;
        assert_eq!(Bot::default().act(&l, exit).weapon, None);
        l.arsenal.weapons[1].ammo = 0;
        assert_eq!(Bot::default().act(&l, exit).weapon, None, "стрелять нечем");
        l.arsenal.weapons[0].ammo = 1;
        assert_eq!(Bot::default().act(&l, exit).weapon, Some(0));
    }

    #[test]
    fn press_turns_to_yaw_and_holds_keys() {
        let mut input = Input::default();
        let action = Action {
            yaw: 1.0,
            walk: true,
            fire: true,
            weapon: Some(1),
        };
        press(action, 0.25, &mut input);
        let mut player = Player::new(Vec2::ZERO, 0.25);
        player.yaw -= input.mouse_delta().x * SENSITIVITY;
        assert!(near(player.yaw, 1.0));
        assert_eq!(input.mouse_delta().y, 0.0);
        assert!(input.is_down(KeyCode::KeyW) && input.mouse_down(MouseButton::Left));
        assert!(input.just_pressed(KeyCode::Digit2) && !input.is_down(KeyCode::Digit1));

        input.end_tick();
        let action = Action {
            walk: false,
            fire: false,
            weapon: Some(0),
            ..action
        };
        press(action, 1.0, &mut input);
        assert_eq!(input.mouse_delta().x, 0.0);
        assert!(!input.is_down(KeyCode::KeyW) && !input.mouse_down(MouseButton::Left));
        assert!(input.just_pressed(KeyCode::Digit1) && !input.is_down(KeyCode::Digit2));
    }
}
