use stapel::{Handle, Model};

/// Вагонов за локомотивом не больше.
pub(crate) const MAX_CARS: usize = 5;
/// Зазор между кузовами соседних машин: наименьший, при котором они не касаются на `corner-large`.
pub(crate) const GAP: f32 = 0.5;

/// Машина состава; перед — +Z модели.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Car {
    pub(crate) model: Handle<Model>,
    /// Габарит модели по z.
    pub(crate) length: f32,
    /// z переднего края в осях модели.
    pub(crate) front: f32,
    /// Отступы передней и задней тележки от переднего края.
    pub(crate) bogies: [f32; 2],
}

impl Car {
    /// `None` без габаритов или тележек: модель не загрузилась.
    pub(crate) fn new(handle: Handle<Model>, model: &Model) -> Option<Self> {
        let bounds = model.bounds()?;
        let z = |name: &'static str| {
            model
                .nodes
                .iter()
                .zip(&model.rest.globals)
                .filter(move |(n, _)| n.name == name)
                .map(|(_, g)| g.w_axis.z)
        };
        let front = z("wheels-front").next()?;
        let back = z("wheels-back").next().or_else(|| {
            let min = z("wheel").reduce(f32::min)?;
            let max = z("wheel").reduce(f32::max)?;
            Some(f32::midpoint(min, max))
        })?;
        Some(Self {
            model: handle,
            length: bounds.max.z - bounds.min.z,
            front: bounds.max.z,
            bogies: [bounds.max.z - front, bounds.max.z - back],
        })
    }
}

/// От переднего края первой машины до заднего края последней.
pub(crate) fn len(cars: &[Car]) -> f32 {
    cars.iter().map(|c| c.length).sum::<f32>() + GAP * cars.len().saturating_sub(1) as f32
}

/// Длина локомотива и `MAX_CARS` вагонов, если все — самые длинные из `catalog`.
pub(crate) fn max_len(catalog: &[Car]) -> f32 {
    (catalog.iter())
        .max_by(|a, b| a.length.total_cmp(&b.length))
        .map_or(0.0, |&c| len(&[c; MAX_CARS + 1]))
}

/// Машина игры по имени из `MODELS`.
#[cfg(test)]
pub(crate) fn load(assets: &mut stapel::Assets, name: &str) -> Car {
    let h = assets.load_model(format!("{}{name}.glb", crate::ASSETS));
    Car::new(h, assets.model(h)).unwrap_or_else(|| panic!("{name}"))
}

/// Все машины игры, как в `init`.
#[cfg(test)]
pub(crate) fn all(assets: &mut stapel::Assets) -> Vec<Car> {
    let models = crate::load_models(assets);
    crate::catalog(&models, assets).into_values().collect()
}

#[cfg(test)]
mod tests {
    use stapel::Assets;

    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn length_and_bogies_from_front_edge() {
        let mut assets = Assets::default();
        let table = [
            ("train/train-carriage-box", 2.7, 1.35, [0.75, 1.95]),
            ("train/train-diesel-box-a", 2.6232, 1.4732, [0.7732, 1.8732]),
            ("train/train-locomotive-a", 2.6, 1.3, [0.7, 1.6928]),
            ("train/train-electric-double-a", 2.5, 1.25, [1.05, 1.85]),
        ];
        for (name, length, front, bogies) in table {
            let c = load(&mut assets, name);
            assert!(
                near(c.length, length) && near(c.front, front),
                "{name}: {c:?}"
            );
            assert!(
                near(c.bogies[0], bogies[0]) && near(c.bogies[1], bogies[1]),
                "{name}: {c:?}"
            );
        }
    }

    #[test]
    fn every_car_of_game_has_bogies_inside_body() {
        for c in all(&mut Assets::default()) {
            assert!(
                0.0 < c.bogies[0] && c.bogies[0] < c.bogies[1] && c.bogies[1] < c.length,
                "{c:?}"
            );
        }
    }

    #[test]
    fn no_car_without_bounds_or_front_bogie() {
        let mut assets = Assets::default();
        let h = assets.load_model(format!("{}train/railroad-straight.glb", crate::ASSETS));
        assert_eq!(Car::new(h, assets.model(h)), None);
        assert_eq!(Car::new(h, &Model::default()), None);
    }

    #[test]
    fn len_adds_gap_between_cars() {
        let mut assets = Assets::default();
        let a = load(&mut assets, "train/train-diesel-a");
        let b = load(&mut assets, "train/train-carriage-tank");
        assert_eq!(len(&[]), 0.0);
        assert_eq!(len(&[a]), a.length);
        assert!(near(len(&[a, b, b]), a.length + 2.0 * b.length + 2.0 * GAP));
    }

    #[test]
    fn max_len_grows_with_max_cars_of_longest() {
        let mut assets = Assets::default();
        let catalog = all(&mut assets);
        let longest = load(&mut assets, "train/train-electric-bullet-a");
        assert!(catalog.iter().all(|c| c.length <= longest.length));
        let want = longest.length * (MAX_CARS + 1) as f32 + GAP * MAX_CARS as f32;
        assert!(near(max_len(&catalog), want));
        let short = &catalog[..1];
        assert!(near(max_len(short), len(&[short[0]; MAX_CARS + 1])));
        assert_eq!(max_len(&[]), 0.0);
    }
}
