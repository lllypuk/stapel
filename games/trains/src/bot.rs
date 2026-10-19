//! Бот сквозных тестов: ведёт состав через `Input`, как игрок.

use std::collections::VecDeque;

use engine::{ElementState, Input, KeyCode, Vec2};

use crate::Trains;
use crate::piece::Piece;
use crate::switch;
use crate::track::Track;

/// Конец отрезка поездки.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Until {
    /// Состав целиком проехал стрелку в этой точке.
    Clear(Vec2),
    /// Состав упёрся: тупик или стрелка на другой ветке.
    Stop,
}

/// Ход в одну сторону до `until`, затем торможение до остановки.
#[derive(Clone, Debug)]
pub(crate) struct Leg {
    pub(crate) back: bool,
    /// Стрелки по точке: `true` — на S-деталь.
    pub(crate) switches: Vec<(Vec2, bool)>,
    pub(crate) until: Until,
}

/// Ввод бота на тик.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Action {
    pub(crate) pedal: f32,
    /// E — стрелка впереди.
    pub(crate) throw: bool,
    /// Tab — закрыть депо.
    pub(crate) tab: bool,
}

/// Проходит отрезки по очереди; стрелку из отрезка переводит E, когда она впереди.
#[derive(Debug)]
pub(crate) struct Bot {
    legs: VecDeque<Leg>,
    /// Педаль нажата в этом отрезке.
    pushed: bool,
    /// Стрелка `Until::Clear` была под составом.
    crossing: bool,
    braking: bool,
}

impl Bot {
    pub(crate) fn new(legs: Vec<Leg>) -> Self {
        Self {
            legs: legs.into(),
            pushed: false,
            crossing: false,
            braking: false,
        }
    }

    /// Все отрезки пройдены, состав стоит.
    pub(crate) fn done(&self) -> bool {
        self.legs.is_empty()
    }

    /// Ввод на тик; открытое депо бот закрывает Tab.
    pub(crate) fn act(&mut self, g: &Trains) -> Action {
        if g.depot.open {
            return Action {
                tab: true,
                ..Action::default()
            };
        }
        let (track, train) = (&g.track, &g.train);
        loop {
            let Some(leg) = self.legs.front() else {
                return Action::default();
            };
            if self.braking {
                if train.speed != 0.0 {
                    return Action {
                        pedal: -train.speed.signum(),
                        ..Action::default()
                    };
                }
                self.legs.pop_front();
                (self.pushed, self.crossing, self.braking) = (false, false, false);
                continue;
            }
            let arrived = match leg.until {
                Until::Stop => self.pushed && train.speed == 0.0,
                Until::Clear(at) => {
                    let under = switch::under(track, train, switch_at(track, at));
                    self.crossing |= under;
                    self.crossing && !under
                }
            };
            if arrived {
                self.braking = true;
                continue;
            }
            self.pushed = true;
            let throw = switch::ahead(track, train).is_some_and(|s| {
                let sw = &track.switches[s];
                let curve = track.edges[sw.branches[sw.active]].rail.piece == Piece::Curve;
                (leg.switches.iter())
                    .any(|&(at, want)| track.ports[sw.port].pos == at && want != curve)
            });
            return Action {
                pedal: if leg.back { -1.0 } else { 1.0 },
                throw,
                tab: false,
            };
        }
    }
}

/// Стрелка в точке `at`.
pub(crate) fn switch_at(track: &Track, at: Vec2) -> usize {
    (track.switches.iter())
        .position(|s| track.ports[s.port].pos == at)
        .unwrap_or_else(|| panic!("нет стрелки в {at}"))
}

/// Действие — в события ввода; E и Tab отпускаются и жмутся заново.
pub(crate) fn press(action: Action, input: &mut Input) {
    let state = |on: bool| {
        if on {
            ElementState::Pressed
        } else {
            ElementState::Released
        }
    };
    input.on_key(KeyCode::KeyW, state(action.pedal > 0.0), false);
    input.on_key(KeyCode::KeyS, state(action.pedal < 0.0), false);
    for (key, on) in [(KeyCode::KeyE, action.throw), (KeyCode::Tab, action.tab)] {
        input.on_key(key, ElementState::Released, false);
        input.on_key(key, state(on), false);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use engine::{Context, Game};

    use super::*;
    use crate::consist;
    use crate::depot;
    use crate::track::level01::{A, B, C, D, T};
    use crate::track::{MARGIN, Pass};
    use crate::train::Train;
    use crate::{TICK_HZ, in_depot};

    /// Тиков на один `run`: самый долгий — около 2300.
    const LIMIT: u32 = 3000;

    fn leg(back: bool, switches: &[(Vec2, bool)], until: Until) -> Leg {
        Leg {
            back,
            switches: switches.to_vec(),
            until,
        }
    }

    fn tick(g: &mut Trains, ctx: &mut Context) {
        g.update(ctx);
        ctx.input.end_tick();
    }

    /// Маршрут — путь графа по нынешним стрелкам, концы на своих рёбрах, длина — длина состава.
    fn on_track(t: &Track, tr: &Train) {
        let r = &tr.route;
        for (&a, &b) in r.iter().zip(r.iter().skip(1)) {
            assert_eq!(t.next(a), Some(b), "{tr:?}");
        }
        let len = |p: Pass| t.edges[p.0].len();
        let (first, last) = (r[0], r[r.len() - 1]);
        assert!((0.0..=len(first)).contains(&tr.tail), "{tr:?}");
        assert!((0.0..=len(last)).contains(&tr.head), "{tr:?}");
        let along = r.iter().map(|&p| len(p)).sum::<f32>() - tr.tail - (len(last) - tr.head);
        assert!(
            (along - consist::len(&tr.cars)).abs() < 1e-3,
            "{along}: {tr:?}"
        );
    }

    /// Бот проходит `legs`, состав каждый тик на пути; рёбра, по которым он прошёл.
    fn run(g: &mut Trains, ctx: &mut Context, legs: Vec<Leg>) -> BTreeSet<usize> {
        let mut bot = Bot::new(legs);
        let mut seen = BTreeSet::new();
        for _ in 0..LIMIT {
            if bot.done() {
                return seen;
            }
            press(bot.act(g), &mut ctx.input);
            tick(g, ctx);
            on_track(&g.track, &g.train);
            seen.extend(g.train.route.iter().map(|p| p.0));
        }
        panic!("бот не доехал за {LIMIT} тиков: {:?}", g.train);
    }

    /// Обгонный путь и отросток к T.
    fn siding(t: &Track, e: usize) -> bool {
        t.edges[e].rail.cell.y < 0
    }

    /// Прямые петли между A и B.
    fn between_a_and_b(t: &Track, e: usize) -> bool {
        let r = t.edges[e].rail;
        r.piece == Piece::Straight && r.cell.y == 0 && (4..=14).contains(&r.cell.x)
    }

    fn in_depot_path(t: &Track, e: usize) -> bool {
        t.depot.iter().any(|&(d, _)| d == e)
    }

    #[test]
    fn loops_with_switch_a_both_ways() {
        for curve in [false, true] {
            let mut ctx = Context::default();
            let mut g = Trains::init(&mut ctx);
            let switches = [(A, curve), (C, false), (B, curve), (D, false)];
            let seen = run(
                &mut g,
                &mut ctx,
                vec![
                    leg(false, &[], Until::Clear(D)),
                    leg(false, &switches, Until::Clear(D)),
                ],
            );
            let t = &g.track;
            assert!(seen.iter().any(|&e| in_depot_path(t, e)));
            assert_eq!(seen.iter().any(|&e| siding(t, e)), curve, "A {curve}");
            assert_eq!(seen.iter().any(|&e| between_a_and_b(t, e)), !curve);
            assert!(!g.train.route.iter().any(|&(e, _)| in_depot_path(t, e)));
        }
    }

    #[test]
    fn backs_through_a_from_siding_onto_loop() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let legs = vec![
            leg(false, &[], Until::Clear(D)),
            leg(false, &[(A, true)], Until::Clear(A)),
            leg(true, &[], Until::Clear(A)),
        ];
        let seen = run(&mut g, &mut ctx, legs);
        let t = &g.track;
        assert!(seen.iter().any(|&e| siding(t, e)));
        assert!(!seen.iter().any(|&e| between_a_and_b(t, e)));
        assert!(g.train.reverse);
        let route = || g.train.route.iter().map(|p| p.0);
        assert!(route().all(|e| !siding(&g.track, e) && !in_depot_path(&g.track, e)));
        let seen = run(
            &mut g,
            &mut ctx,
            vec![leg(false, &[(A, false)], Until::Clear(B))],
        );
        let t = &g.track;
        assert!(seen.iter().any(|&e| between_a_and_b(t, e)));
        assert!(!seen.iter().any(|&e| siding(t, e)));
    }

    #[test]
    fn returns_to_depot_through_d_backwards() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let legs = vec![
            leg(false, &[], Until::Clear(D)),
            leg(false, &[(D, false)], Until::Clear(D)),
            leg(true, &[(D, true)], Until::Stop),
        ];
        run(&mut g, &mut ctx, legs);
        let want = in_depot(&g.track, &g.cars, &g.depot.consist);
        assert_eq!(g.train.route, want.route);
        assert!((g.train.tail - want.tail).abs() < 1e-3, "{:?}", g.train);
        assert!((g.train.head - want.head).abs() < 1e-3, "{:?}", g.train);
        assert!(depot::can_open(&g.track, &g.train));
        press(
            Action {
                tab: true,
                ..Action::default()
            },
            &mut ctx.input,
        );
        tick(&mut g, &mut ctx);
        assert!(g.depot.open);
    }

    /// Наименьший запас точек кузовов до `at` вдоль `dir`.
    fn short_of(g: &Trains, at: Vec2, dir: Vec2) -> f32 {
        let (placed, _) = g.train.cars_at(&g.track);
        (placed.iter().zip(&g.train.cars))
            .flat_map(|(p, c)| [c.front, c.front - c.length].map(|z| p.pos + p.dir * z))
            .map(|p| (at - p).dot(dir))
            .fold(f32::MAX, f32::min)
    }

    /// Упёршийся состав не трогается и под газом.
    fn stays_under_gas(g: &mut Trains, ctx: &mut Context) {
        let stood = g.train.clone();
        for _ in 0..TICK_HZ {
            press(
                Action {
                    pedal: 1.0,
                    ..Action::default()
                },
                &mut ctx.input,
            );
            tick(g, ctx);
            assert_eq!(g.train, stood);
        }
    }

    #[test]
    fn stops_whole_before_dead_end_t() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let legs = vec![
            leg(false, &[], Until::Clear(D)),
            leg(false, &[(A, true), (C, true)], Until::Stop),
        ];
        run(&mut g, &mut ctx, legs);
        let last = g.train.route[g.train.route.len() - 1];
        assert_eq!(g.track.edges[last.0].rail.point_at(4.0).0, T);
        assert!(
            (g.train.head - (4.0 - MARGIN)).abs() < 1e-4,
            "{:?}",
            g.train
        );
        let short = short_of(&g, T, Vec2::X);
        assert!((short - MARGIN).abs() < 1e-3, "{short}");
        stays_under_gas(&mut g, &mut ctx);
    }

    #[test]
    fn stops_whole_before_b_on_other_branch() {
        let mut ctx = Context::default();
        let mut g = Trains::init(&mut ctx);
        let legs = vec![
            leg(false, &[], Until::Clear(D)),
            leg(false, &[(A, true), (C, false), (B, false)], Until::Stop),
        ];
        run(&mut g, &mut ctx, legs);
        let last = g.train.route[g.train.route.len() - 1];
        let edge = &g.track.edges[last.0];
        assert_eq!(
            (edge.rail.piece, edge.rail.point_at(edge.len()).0),
            (Piece::Curve, B)
        );
        assert!(
            (g.train.head - (edge.len() - MARGIN)).abs() < 1e-4,
            "{:?}",
            g.train
        );
        let short = short_of(&g, B, Vec2::X);
        assert!(short > MARGIN - 0.05, "{short}");
        stays_under_gas(&mut g, &mut ctx);
    }

    #[test]
    fn press_holds_pedal_and_taps_e_and_tab() {
        let mut input = Input::default();
        let down = |i: &Input| [KeyCode::KeyW, KeyCode::KeyS].map(|k| i.is_down(k));
        let tapped = |i: &Input| [KeyCode::KeyE, KeyCode::Tab].map(|k| i.just_pressed(k));
        let both = Action {
            pedal: 1.0,
            throw: true,
            tab: true,
        };
        press(both, &mut input);
        assert_eq!(
            (down(&input), tapped(&input)),
            ([true, false], [true, true])
        );
        input.end_tick();
        press(
            Action {
                pedal: -1.0,
                ..both
            },
            &mut input,
        );
        assert_eq!(
            (down(&input), tapped(&input)),
            ([false, true], [true, true])
        );
        input.end_tick();
        press(Action::default(), &mut input);
        assert_eq!(
            (down(&input), tapped(&input)),
            ([false, false], [false, false])
        );
    }
}
