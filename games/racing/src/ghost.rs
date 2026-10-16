//! Призрак лучшего круга: запись начала модели по тикам и проигрывание.

use std::mem;

use engine::{Color, Mat4, ModelDraw};

use crate::layout::Models;
use crate::race::Pass;

/// Голубой, полупрозрачный — дизеринг.
pub(crate) const TINT: Color = Color {
    r: 0.3,
    g: 0.7,
    b: 1.0,
    a: 0.4,
};

/// Начало модели после тика круга.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    pub(crate) transform: Mat4,
    /// Перед тиком был респаун: от прежнего сэмпла не интерполируется.
    pub(crate) cut: bool,
}

/// Сэмпл `k` — после тика `k` от пересечения старта, последний — тик следующего пересечения.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lap {
    pub(crate) car: String,
    pub(crate) samples: Vec<Sample>,
}

impl Lap {
    /// Начало модели на тике `t` круга между тиками `t - 1` и `t`; `None` после конца круга.
    pub(crate) fn transform(&self, t: u64, alpha: f32) -> Option<Mat4> {
        let t = usize::try_from(t).ok()?;
        let cur = self.samples.get(t)?;
        if t == 0 || cur.cut {
            return Some(cur.transform);
        }
        let (_, r0, p0) = self.samples[t - 1]
            .transform
            .to_scale_rotation_translation();
        let (_, r1, p1) = cur.transform.to_scale_rotation_translation();
        Some(Mat4::from_rotation_translation(
            r0.slerp(r1, alpha),
            p0.lerp(p1, alpha),
        ))
    }
}

/// Запись текущего круга.
#[derive(Default)]
pub(crate) struct Recorder {
    /// `None` до пересечения старта.
    lap: Option<Lap>,
    cut: bool,
}

impl Recorder {
    /// Следующий сэмпл — после респауна.
    pub(crate) fn cut(&mut self) {
        self.cut = true;
    }

    /// Круг заново: пишется с пересечения старта.
    pub(crate) fn restart(&mut self) {
        *self = Self::default();
    }

    /// Сэмпл тика машины `car` с началом модели `transform`; отдаёт законченный круг.
    pub(crate) fn tick(&mut self, car: &str, transform: Mat4, pass: Option<Pass>) -> Option<Lap> {
        let sample = Sample {
            transform,
            cut: mem::take(&mut self.cut),
        };
        if let Some(lap) = &mut self.lap {
            lap.samples.push(sample);
        }
        let pass = pass.filter(|p| p.checkpoint == 0)?;
        let new = Lap {
            car: car.into(),
            samples: vec![sample],
        };
        let done = self.lap.replace(new);
        pass.lap.and(done)
    }
}

/// Лучший круг трассы.
#[derive(Default)]
pub(crate) struct Ghost {
    pub(crate) best: Option<Lap>,
}

impl Ghost {
    /// Круг становится лучшим, только если он быстрее.
    pub(crate) fn offer(&mut self, lap: Lap) {
        if self
            .best
            .as_ref()
            .is_none_or(|b| lap.samples.len() < b.samples.len())
        {
            self.best = Some(lap);
        }
    }

    /// Модель записанной машины на тике `t` круга, колёса в позе покоя.
    pub(crate) fn draw(&self, t: u64, alpha: f32, models: &Models) -> Option<ModelDraw> {
        let lap = self.best.as_ref()?;
        Some(ModelDraw {
            model: models.get(&lap.car),
            transform: lap.transform(t, alpha)?,
            pose: None,
            tint: TINT,
        })
    }
}

#[cfg(test)]
mod tests {
    use engine::{Assets, Quat, Vec3, vec3};

    use super::*;

    /// Сэмпл `k`: сдвиг на `k` по X и поворот на `0,1 · k` вокруг Y.
    fn at(k: u64) -> Mat4 {
        let k = k as f32;
        Mat4::from_rotation_translation(Quat::from_rotation_y(0.1 * k), vec3(k, 0.0, 0.0))
    }

    fn pass(checkpoint: usize, lap: Option<u64>) -> Pass {
        Pass { checkpoint, lap }
    }

    /// Круг в `ticks` тиков: старт на тике 0, конец на тике `ticks`.
    fn record(r: &mut Recorder, car: &str, ticks: u64) -> Lap {
        assert_eq!(r.tick(car, at(0), Some(pass(0, None))), None);
        for k in 1..ticks {
            assert_eq!(r.tick(car, at(k), None), None);
        }
        r.tick(car, at(ticks), Some(pass(0, Some(ticks))))
            .expect("круг")
    }

    /// Разбор `Mat4` на поворот и сдвиг теряет последний бит.
    fn close(got: Option<Mat4>, want: Mat4) -> bool {
        got.is_some_and(|m| m.abs_diff_eq(want, 1e-5))
    }

    fn lap(car: &str, ticks: u64) -> Lap {
        record(&mut Recorder::default(), car, ticks)
    }

    #[test]
    fn sample_k_is_state_after_tick_k_from_start_crossing() {
        let mut r = Recorder::default();
        for k in 0..5 {
            assert_eq!(r.tick("race", at(100 + k), None), None, "до старта");
        }
        let lap = record(&mut r, "race", 4);
        assert_eq!(lap.car, "race");
        let samples: Vec<_> = lap.samples.iter().map(|s| s.transform).collect();
        assert_eq!(samples, (0..=4).map(at).collect::<Vec<_>>());
        assert!(lap.samples.iter().all(|s| !s.cut));
        let next = r.lap.as_ref().expect("следующий круг");
        assert_eq!(
            next.samples,
            [lap.samples[4]],
            "конец круга — начало следующего"
        );
    }

    #[test]
    fn other_checkpoints_and_first_start_finish_no_lap() {
        let mut r = Recorder::default();
        assert_eq!(r.tick("race", at(0), Some(pass(1, None))), None);
        assert!(r.lap.is_none(), "чекпоинт 1 — не старт");
        assert_eq!(r.tick("race", at(0), Some(pass(0, None))), None);
        assert_eq!(r.tick("race", at(1), Some(pass(1, None))), None);
        assert_eq!(r.lap.as_ref().map(|l| l.samples.len()), Some(2));
        r.restart();
        assert!(r.lap.is_none());
        assert_eq!(
            r.tick("race", at(0), Some(pass(0, Some(5)))),
            None,
            "без записи"
        );
    }

    #[test]
    fn playback_interpolates_previous_to_current_except_at_zero() {
        let lap = lap("race", 4);
        assert_eq!(lap.transform(0, 0.5), Some(at(0)));
        assert!(close(lap.transform(4, 1.0), at(4)));
        let mid =
            Mat4::from_rotation_translation(Quat::from_rotation_y(0.125), vec3(1.25, 0.0, 0.0));
        assert!(close(lap.transform(2, 0.25), mid));
        assert!(close(lap.transform(1, 0.0), at(0)));
        assert_eq!(lap.transform(5, 0.0), None, "после конца круга");
        assert_eq!(lap.transform(u64::MAX, 0.0), None);
    }

    #[test]
    fn respawn_cuts_interpolation_once() {
        let mut r = Recorder::default();
        r.tick("race", at(0), Some(pass(0, None)));
        r.tick("race", at(1), None);
        r.cut();
        r.tick("race", at(9), None);
        r.tick("race", at(10), None);
        let lap = r
            .tick("race", at(11), Some(pass(0, Some(4))))
            .expect("круг");
        let cuts: Vec<_> = lap.samples.iter().map(|s| s.cut).collect();
        assert_eq!(cuts, [false, false, true, false, false]);
        assert_eq!(lap.transform(2, 0.5), Some(at(9)));
        let after = lap.transform(3, 0.5).expect("тик 3");
        assert!((after.transform_point3(Vec3::ZERO)).abs_diff_eq(vec3(9.5, 0.0, 0.0), 1e-5));
    }

    #[test]
    fn best_is_replaced_only_by_faster_lap() {
        let mut g = Ghost::default();
        g.offer(lap("race", 5));
        g.offer(lap("sedan-sports", 6));
        g.offer(lap("sedan-sports", 5));
        assert_eq!(g.best, Some(lap("race", 5)));
        g.offer(lap("sedan-sports", 4));
        assert_eq!(g.best, Some(lap("sedan-sports", 4)));
    }

    #[test]
    fn draw_uses_recorded_car_model_tint_and_rest_pose() {
        let mut assets = Assets::default();
        let models = Models::load(&mut assets);
        let mut g = Ghost::default();
        assert_eq!(g.draw(0, 0.0, &models), None);
        g.offer(lap("sedan-sports", 3));
        let draw = g.draw(3, 1.0, &models).expect("призрак");
        assert_eq!(draw.model, models.get("sedan-sports"));
        assert!(close(Some(draw.transform), at(3)));
        assert_eq!((draw.pose, draw.tint), (None, TINT));
        const { assert!(TINT.a < 1.0) };
        assert_eq!(g.draw(4, 0.0, &models), None);
    }
}
